"""Check the attestations an in-toto grate made for a preload bundle.

Usage: verify-attest.py <attest dir> <bundle>

<attest dir> holds signer.pub.json and <n>-<step>.intoto.json as dumped by
the IMFS grate. For each envelope this checks, against the bundle the host
packed:
  - the signature (ECDSA P-256 over SHA-256 of the DSSE PAE, via openssl)
    with the published key, whose keyid is the SHA-256 of its canonical JSON;
  - that the statement's subjects are exactly the files of archive <n>, with
    their SHA-256, and that the archive digest matches;
  - that each statement after the first names the previous envelope's
    SHA-256.
"""
import base64, hashlib, json, os, re, struct, subprocess, sys, tempfile

PAYLOAD_TYPE = "application/vnd.in-toto+json"
PREDICATE_TYPE = "https://lind-project.org/attestation/imfs-preload/v1"
HEADER = 56


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


def archives(bundle):
    """[(payload sha256, [(path, sha256)])] for each archive of the bundle."""
    out, pos = [], 0
    while pos < len(bundle):
        magic, version, count, plen = struct.unpack_from("<8sIIQ", bundle, pos)
        assert magic == b"LINDPRLD" and version == 2, "not a preload archive"
        digest = bundle[pos + 24:pos + HEADER].hex()
        payload = bundle[pos + HEADER:pos + HEADER + plen]
        assert hashlib.sha256(payload).hexdigest() == digest, "bundle payload digest"
        files, p = [], 0
        for _ in range(count):
            path_len, _mode, data_len = struct.unpack_from("<IIQ", payload, p)
            p += 16
            path = payload[p:p + path_len].decode()
            p += path_len
            files.append((path, hashlib.sha256(payload[p:p + data_len]).hexdigest()))
            p += data_len
        out.append((digest, files))
        pos += HEADER + plen
    return out


def verify_sig(pem, message, der):
    with tempfile.TemporaryDirectory() as d:
        for name, data in (("pub.pem", pem.encode()), ("msg", message), ("sig", der)):
            with open(os.path.join(d, name), "wb") as f:
                f.write(data)
        r = subprocess.run(["openssl", "dgst", "-sha256", "-verify", os.path.join(d, "pub.pem"),
                            "-signature", os.path.join(d, "sig"), os.path.join(d, "msg")],
                           capture_output=True, text=True)
        return r.returncode == 0 and "Verified OK" in r.stdout


def main():
    attest_dir, bundle_path = sys.argv[1], sys.argv[2]
    key = json.load(open(os.path.join(attest_dir, "signer.pub.json")))
    keyid = key.pop("keyid")
    assert hashlib.sha256(canonical(key).encode()).hexdigest() == keyid, "keyid"
    pem = key["keyval"]["public"]
    print(f"signer: {key['scheme']} keyid {keyid[:16]}...")

    bundle_archives = archives(open(bundle_path, "rb").read())
    names = sorted((f for f in os.listdir(attest_dir) if re.match(r"\d+-.*\.intoto\.json$", f)),
                   key=lambda f: int(f.split("-")[0]))
    assert len(names) == len(bundle_archives), f"{len(names)} attestations for {len(bundle_archives)} archives"

    previous, ok = None, True
    for name in names:
        raw = open(os.path.join(attest_dir, name), "rb").read()
        env = json.loads(raw)
        payload = base64.b64decode(env["payload"])
        sig = env["signatures"][0]
        pae = b"DSSEv1 %d %s %d " % (len(PAYLOAD_TYPE), PAYLOAD_TYPE.encode(), len(payload)) + payload
        good_sig = env["payloadType"] == PAYLOAD_TYPE and sig["keyid"] == keyid and \
            verify_sig(pem, pae, base64.b64decode(sig["sig"]))

        st = json.loads(payload)
        pred = st["predicate"]
        index = pred["index"]
        digest, files = bundle_archives[index - 1]
        subjects = [(s["name"], s["digest"]["sha256"]) for s in st["subject"]]
        good_subjects = st["predicateType"] == PREDICATE_TYPE and subjects == files and \
            pred["archive"]["sha256"] == digest
        good_chain = (pred.get("previous", {}).get("sha256") == previous)
        previous = hashlib.sha256(raw).hexdigest()

        print(f"attestation {index} ({pred['step']}): {len(subjects)} files, "
              f"signature {'OK' if good_sig else 'BAD'}, "
              f"subjects {'match the bundle' if good_subjects else 'DO NOT MATCH'}, "
              f"chain {'OK' if good_chain else 'BROKEN'}")
        ok &= good_sig and good_subjects and good_chain
    print("attestations verified" if ok else "ATTESTATION CHECK FAILED")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
