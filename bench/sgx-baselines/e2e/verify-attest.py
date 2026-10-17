"""Check the evidence of a TriSeal build against the bundle and the outputs.

Usage: verify-attest.py <dump dir> <bundle>

<dump dir> is the unpacked dump of the keep: the output files, and
evidence/evidence.json, which the in-toto grate (the recorder) assembled from
its own state at the end of the run:

  recorder.publicKey   the key the recorder made in the keep
  chain                the envelopes it signed, in order (exact bytes):
                       one per archive of the bundle, then the products
  runtime              what exec-wasmtime answered when the recorder handed
                       in its key and the hash of its last envelope: the
                       identity key, the TEE report (SGX quote), run-start
                       and run-end, signed with the identity key

This checks that the whole chain hangs off the quote:
  - the quote's report data is the SHA-256 of the identity key (SPKI);
  - the identity key signed run-start and run-end (ECDSA P-256 over SHA-256
    of the DSSE PAE, via openssl), which belong to one run, run-end naming
    the SHA-256 of run-start;
  - run-end names the recorder's key and the SHA-256 of the last envelope,
    and the number of envelopes;
  - every envelope is signed by the recorder's key and names the SHA-256 of
    the one before;
  - the statements' subjects are exactly the files of each bundle archive,
    and the products statement's are exactly the files in <dump dir> (other
    than the evidence), with their SHA-256.

It does not check the quote's own signature and certificate chain up to
Intel (DCAP collateral); it prints MRENCLAVE and MRSIGNER for the verifier
to compare with the expected keep.

A dump dir from an older recorder (attest/ with signer.pub.json and
<n>-<step>.intoto.json, nothing anchored to the keep) is checked the old way
and reported as not anchored.
"""
import base64, hashlib, json, os, re, struct, subprocess, sys, tempfile

PAYLOAD_TYPE = "application/vnd.in-toto+json"
PRELOAD = "https://lind-project.org/attestation/imfs-preload/v1"
PRODUCTS = "https://lind-project.org/attestation/imfs-products/v1"
RUN_START = "https://lind-project.org/attestation/lind-wasm/run-start/v1"
RUN_END = "https://lind-project.org/attestation/lind-wasm/run-end/v1"
EVIDENCE = os.path.join("evidence", "evidence.json")
HEADER = 56
# DER prefix of a P-256 SubjectPublicKeyInfo, before the 65-byte point.
P256_SPKI_PREFIX = bytes.fromhex("3059301306072a8648ce3d020106082a8648ce3d030107034200")


class Bad(Exception):
    pass


def check(cond, what):
    if not cond:
        raise Bad(what)


def canonical(value):
    """securesystemslib canonical JSON."""
    if isinstance(value, dict):
        return "{" + ",".join(canonical(k) + ":" + canonical(value[k]) for k in sorted(value)) + "}"
    if isinstance(value, list):
        return "[" + ",".join(canonical(v) for v in value) + "]"
    if isinstance(value, str):
        return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'
    if isinstance(value, bool):
        return "true" if value else "false"
    if value is None:
        return "null"
    return str(value)


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def archives(bundle):
    """[(payload sha256, [(path, sha256)])] for each archive of the bundle."""
    out, pos = [], 0
    while pos < len(bundle):
        magic, version, count, plen = struct.unpack_from("<8sIIQ", bundle, pos)
        check(magic == b"LINDPRLD" and version == 2, "not a preload archive")
        digest = bundle[pos + 24:pos + HEADER].hex()
        payload = bundle[pos + HEADER:pos + HEADER + plen]
        check(sha256(payload) == digest, "bundle payload digest")
        files, p = [], 0
        for _ in range(count):
            path_len, _mode, data_len = struct.unpack_from("<IIQ", payload, p)
            p += 16
            path = payload[p:p + path_len].decode()
            p += path_len
            files.append((path, sha256(payload[p:p + data_len])))
            p += data_len
        out.append((digest, files))
        pos += HEADER + plen
    return out


def openssl_verify(pem, message, der):
    with tempfile.TemporaryDirectory() as d:
        for name, data in (("pub.pem", pem.encode()), ("msg", message), ("sig", der)):
            with open(os.path.join(d, name), "wb") as f:
                f.write(data)
        r = subprocess.run(["openssl", "dgst", "-sha256", "-verify", os.path.join(d, "pub.pem"),
                            "-signature", os.path.join(d, "sig"), os.path.join(d, "msg")],
                           capture_output=True, text=True)
        return r.returncode == 0 and "Verified OK" in r.stdout


class Key:
    """A public key in in-toto / securesystemslib JSON: the grate's (PEM) or
    the in-toto crate's (hex of the P-256 point)."""

    def __init__(self, obj, what):
        obj = dict(obj)
        self.keyid = obj.pop("keyid", None)
        keyval = dict(obj.get("keyval", {}))
        keyval.pop("private", None)  # the in-toto crate writes an empty one
        obj["keyval"] = keyval
        self.object = obj
        check(obj.get("scheme") == "ecdsa-sha2-nistp256", f"{what}: not an ECDSA P-256 key")
        check(sha256(canonical(obj).encode()) == self.keyid, f"{what}: keyid is not the key's")
        public = keyval["public"]
        if public.startswith("-----BEGIN PUBLIC KEY-----"):
            self.pem = public.strip() + "\n"
            body = "".join(l for l in public.strip().splitlines() if not l.startswith("-----"))
            self.spki = base64.b64decode(body)
        else:
            raw = bytes.fromhex(public)
            self.spki = raw if raw.startswith(b"\x30") else P256_SPKI_PREFIX + raw
            self.pem = ("-----BEGIN PUBLIC KEY-----\n" + base64.encodebytes(self.spki).decode()
                        + "-----END PUBLIC KEY-----\n")

    def open(self, envelope_bytes, what):
        """Check the envelope's signature; return its statement."""
        env = json.loads(envelope_bytes)
        check(env.get("payloadType") == PAYLOAD_TYPE, f"{what}: payload type")
        payload = base64.b64decode(env["payload"])
        sigs = env.get("signatures", [])
        check(len(sigs) == 1 and sigs[0].get("keyid") == self.keyid, f"{what}: not signed by this key")
        pae = b"DSSEv1 %d %s %d " % (len(PAYLOAD_TYPE), PAYLOAD_TYPE.encode(), len(payload)) + payload
        check(openssl_verify(self.pem, pae, base64.b64decode(sigs[0]["sig"])), f"{what}: bad signature")
        return json.loads(payload)


def dumped_files(dump_dir):
    """[(path, sha256)] of the files in the dump, other than the evidence."""
    out = []
    for root, _dirs, files in os.walk(dump_dir):
        for name in files:
            path = os.path.join(root, name)
            rel = os.path.relpath(path, dump_dir)
            if rel == EVIDENCE:
                continue
            out.append((rel, sha256(open(path, "rb").read())))
    return sorted(out)


def der_element(data, pos):
    """(tag, content start, content end) of the DER element at `pos`."""
    check(pos + 2 <= len(data), "truncated DER")
    tag, length = data[pos], data[pos + 1]
    pos += 2
    if length & 0x80:
        n = length & 0x7F
        check(0 < n <= 4 and pos + n <= len(data), "bad DER length")
        length = int.from_bytes(data[pos:pos + n], "big")
        pos += n
    check(pos + length <= len(data), "truncated DER")
    return tag, pos, pos + length


def sgx_quote(report):
    """The quote out of enarx's SGX evidence: DER SEQUENCE { quote OCTET
    STRING, Intel CRLs, Intel TCB info }, the collateral a full quote check
    needs. A bare quote is taken as is."""
    if report[:1] != b"\x30":
        return report
    tag, start, _ = der_element(report, 0)
    check(tag == 0x30, "the report is not SGX evidence")
    tag, start, end = der_element(report, start)
    check(tag == 0x04, "the SGX evidence holds no quote")
    return report[start:end]


def check_quote(report, spki):
    """SGX quote v3: the report data must be SHA-256(SPKI) then zeros."""
    report = sgx_quote(report)
    check(len(report) >= 432, "the report is not an SGX quote")
    version, = struct.unpack_from("<H", report, 0)
    check(version == 3, f"unsupported quote version {version}")
    body = report[48:432]
    report_data = body[320:384]
    check(report_data[:32] == hashlib.sha256(spki).digest() and report_data[32:] == bytes(32),
          "the quote's report data is not the identity key's hash")
    return body[64:96].hex(), body[128:160].hex()


def verify(dump_dir, bundle_path):
    evidence = json.load(open(os.path.join(dump_dir, EVIDENCE)))
    bundle = archives(open(bundle_path, "rb").read())
    out = []

    recorder = Key(evidence["recorder"]["publicKey"], "recorder key")
    chain = evidence["chain"]
    runtime = evidence.get("runtime")
    check(isinstance(chain, list) and chain, "no chain")
    check(runtime, "no run-end from the runtime: the chain is not anchored to the keep")

    # The keep: quote -> identity key -> run-start, run-end.
    identity = Key(runtime["identityKey"], "identity key")
    report = base64.b64decode(runtime["report"])
    mrenclave, mrsigner = check_quote(report, identity.spki)
    out.append(f"quote: report data = SHA-256(identity key {identity.keyid[:16]}...); "
               f"MRENCLAVE {mrenclave} MRSIGNER {mrsigner[:16]}... "
               "(quote signature not checked here)")
    start = identity.open(runtime["runStart"].encode(), "run-start")
    end = identity.open(runtime["runEnd"].encode(), "run-end")
    check(start["predicateType"] == RUN_START and end["predicateType"] == RUN_END, "run-start/run-end types")
    sp, ep = start["predicate"], end["predicate"]
    check(sp["runId"] == ep["runId"] == runtime["runId"], "run-start and run-end are from different runs")
    check(ep["startAttestation"]["sha256"] == sha256(runtime["runStart"].encode()),
          "run-end does not name this run-start")
    check(sp["keep"]["identityKey"]["sha256"] == sha256(identity.spki), "run-start names another identity key")
    check(sp["keep"]["report"]["sha256"] == sha256(report), "run-start names another report")
    out.append(f"run {sp['runId']}: run-start and run-end signed by the identity key, "
               f"workload {start['subject'][0]['digest']['sha256'][:16]}...")

    # run-end -> the recorder's key and the end of its chain.
    check(ep["recorder"]["keyid"] == recorder.keyid, "run-end names another recorder key")
    named = Key(ep["recorder"]["publicKey"], "recorder key in run-end")
    check(named.object == recorder.object, "run-end names another recorder key")
    check(ep["chain"]["links"] == len(chain), "run-end names a chain of another length")
    check(ep["chain"]["head"]["sha256"] == sha256(chain[-1].encode()),
          "run-end names another last envelope")
    out.append(f"run-end names recorder key {recorder.keyid[:16]}... and the last of {len(chain)} envelopes")

    # The chain: one statement per bundle archive, then the products.
    check(len(chain) == len(bundle) + 1, f"{len(chain)} envelopes for {len(bundle)} archives and the products")
    previous = None
    for i, raw in enumerate(chain):
        st = recorder.open(raw.encode(), f"envelope {i + 1}")
        pred = st["predicate"]
        check(pred["index"] == i + 1, f"envelope {i + 1}: index {pred['index']}")
        check(pred.get("previous", {}).get("sha256") == previous, f"envelope {i + 1}: chain broken")
        previous = sha256(raw.encode())
        subjects = [(s["name"], s["digest"]["sha256"]) for s in st["subject"]]
        if i < len(bundle):
            digest, files = bundle[i]
            check(st["predicateType"] == PRELOAD and subjects == files and pred["archive"]["sha256"] == digest,
                  f"envelope {i + 1} ({pred['step']}): subjects do not match the bundle")
            out.append(f"envelope {i + 1} ({pred['step']}): {len(subjects)} files match the bundle")
        else:
            outputs = dumped_files(dump_dir)
            check(st["predicateType"] == PRODUCTS, "the last envelope is not the products")
            check(sorted(subjects) == outputs, "the products do not match the dumped files")
            out.append(f"envelope {i + 1} (products): {len(subjects)} files match the dump, "
                       f"exit status {pred.get('exitStatus')}")
    return out


def verify_legacy(attest_dir, bundle_path):
    key = Key(json.load(open(os.path.join(attest_dir, "signer.pub.json"))), "signer key")
    bundle = archives(open(bundle_path, "rb").read())
    names = sorted((f for f in os.listdir(attest_dir) if re.match(r"\d+-.*\.intoto\.json$", f)),
                   key=lambda f: int(f.split("-")[0]))
    check(len(names) == len(bundle), f"{len(names)} attestations for {len(bundle)} archives")
    out, previous = [], None
    for name in names:
        raw = open(os.path.join(attest_dir, name), "rb").read()
        st = key.open(raw, name)
        pred = st["predicate"]
        digest, files = bundle[pred["index"] - 1]
        subjects = [(s["name"], s["digest"]["sha256"]) for s in st["subject"]]
        check(st["predicateType"] == PRELOAD and subjects == files and pred["archive"]["sha256"] == digest,
              f"{name}: subjects do not match the bundle")
        check(pred.get("previous", {}).get("sha256") == previous, f"{name}: chain broken")
        previous = sha256(raw)
        out.append(f"attestation {pred['index']} ({pred['step']}): {len(subjects)} files match the bundle")
    out.append("WARNING: legacy evidence: the signer key is not anchored to the keep, no products")
    return out


def main():
    target, bundle_path = sys.argv[1], sys.argv[2]
    try:
        if os.path.exists(os.path.join(target, EVIDENCE)):
            lines = verify(target, bundle_path)
        elif os.path.exists(os.path.join(target, "attest", "signer.pub.json")):
            lines = verify_legacy(os.path.join(target, "attest"), bundle_path)
        else:
            lines = verify_legacy(target, bundle_path)
    except (Bad, KeyError, TypeError, ValueError, IndexError) as e:
        print(f"EVIDENCE CHECK FAILED: {e}")
        sys.exit(1)
    for line in lines:
        print(line)
    print("evidence verified")


if __name__ == "__main__":
    main()
