// SPDX-License-Identifier: Apache-2.0

//! in-toto attestations for the lifecycle of a lind-wasm run.
//!
//! Two attestations are produced per workload run: one right before the lind-wasm
//! runtime (`lind_boot::execute_wasmtime`) starts and one right after it returns.
//!
//! * `run-start` records what is about to run: the digests of the Wasm module, the
//!   guest argv/env/preloads, the TEE technology, and the keep identity the run is
//!   bound to.
//! * `run-end` records how the run ended: the exit code or the runtime error, the
//!   wall-clock duration, and the digest of the `run-start` attestation it belongs to.
//!
//! Each attestation is an [in-toto Statement v1](https://github.com/in-toto/attestation/blob/main/spec/v1/statement.md)
//! wrapped in a [DSSE v1 envelope](https://github.com/secure-systems-lab/dsse/blob/master/envelope.md).
//! The envelope is signed with the keep identity key, i.e. the key whose public-key
//! hash is bound into the TEE attestation report, so the in-toto attestation chains
//! to the hardware attestation. Signing keys, key ids and the public-key file format
//! follow the `in-toto` crate (securesystemslib compatible), so the public key can be
//! consumed by the `in-toto-cli` app that runs inside lind-wasm.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use const_oid::db::rfc5912::SECP_256_R_1;
use data_encoding::BASE64;
use enarx_config::InToto;
use in_toto::crypto::{KeyType, PrivateKey, SignatureScheme};
use pkcs8::der::Decode;
use pkcs8::PrivateKeyInfo;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256, Sha512};
use url::Url;

/// `_type` of every statement produced by this module.
pub const STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";

/// DSSE `payloadType` of an in-toto statement.
pub const PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";

/// Predicate type of the attestation emitted right before lind-wasm starts.
pub const PREDICATE_RUN_START: &str = "https://lind-project.org/attestation/lind-wasm/run-start/v1";

/// Predicate type of the attestation emitted right after lind-wasm ends.
pub const PREDICATE_RUN_END: &str = "https://lind-project.org/attestation/lind-wasm/run-end/v1";

/// Subject name of the workload module. This is also guest `argv[0]`.
pub const SUBJECT_NAME: &str = "main.wasm";

/// Facts about the keep that are recorded in the `run-start` predicate.
pub struct KeepInfo<'a> {
    /// TEE technology name (`sgx`, `snp` or `kvm`)
    pub technology: &'a str,
    /// PKCS#8 DER of the keep identity key; used to sign the attestations
    pub identity_key: &'a [u8],
    /// DER `SubjectPublicKeyInfo` of the keep identity key
    pub identity_spki: &'a [u8],
    /// DER certificates issued for the keep identity key (Steward or self-signed)
    pub certificates: &'a [Vec<u8>],
    /// Steward the keep attested to, if any
    pub steward: Option<&'a Url>,
}

/// What lind-wasm is asked to run.
pub struct Invocation<'a> {
    /// Guest argv, `argv[0]` being the module name
    pub argv: &'a [String],
    /// Guest environment
    pub env: &'a HashMap<String, String>,
    /// Modules loaded before the main module (`NAME=PATH`)
    pub preloads: &'a [(String, PathBuf)],
}

/// How the lind-wasm run ended.
pub enum Outcome<'a> {
    /// All cages exited; this is the exit code of the first cage
    Exited(i32),
    /// The runtime failed before producing an exit code
    Failed(&'a anyhow::Error),
}

/// Where finished envelopes go.
enum Sink {
    /// Files in this directory (falls back to stderr when a write fails)
    Dir(PathBuf),
    /// Standard error only
    Stderr,
}

/// Produces the `run-start` and `run-end` attestations of one lind-wasm run.
pub struct Attestor {
    key: PrivateKey,
    signer_is_identity_key: bool,
    run_id: String,
    subject: Value,
    keep: Value,
    sink: Sink,
    echo: bool,
    started_at: Option<DateTime<Utc>>,
    start_sha256: Option<String>,
}

impl Attestor {
    /// Prepares attestation of a run of `wasm` inside the keep described by `keep`.
    ///
    /// Returns `None` when attestation is disabled in the configuration.
    pub fn new(cfg: &InToto, keep: KeepInfo<'_>, wasm: &[u8]) -> Result<Option<Self>> {
        if cfg.disabled {
            return Ok(None);
        }

        let (key, signer_is_identity_key) = signer(keep.identity_key)?;

        let mut id = [0u8; 8];
        getrandom::getrandom(&mut id).map_err(|e| anyhow!("failed to generate run id: {e}"))?;
        let run_id = hex::encode(id);

        let subject = json!({
            "name": SUBJECT_NAME,
            "digest": {
                "sha256": hex::encode(Sha256::digest(wasm)),
                "sha512": hex::encode(Sha512::digest(wasm)),
            },
        });

        let keep = json!({
            "technology": keep.technology,
            "identityKey": { "sha256": hex::encode(Sha256::digest(keep.identity_spki)) },
            "certificates": keep
                .certificates
                .iter()
                .map(|der| json!({ "sha256": hex::encode(Sha256::digest(der)) }))
                .collect::<Vec<_>>(),
            "steward": keep.steward.map(Url::as_str),
        });

        let (sink, echo) = match &cfg.dir {
            Some(dir) => (Sink::Dir(dir.clone()), cfg.stderr),
            None => (Sink::Stderr, true),
        };

        Ok(Some(Self {
            key,
            signer_is_identity_key,
            run_id,
            subject,
            keep,
            sink,
            echo,
            started_at: None,
            start_sha256: None,
        }))
    }

    /// Emits the `run-start` attestation. Call right before lind-wasm starts.
    pub fn start(&mut self, invocation: &Invocation<'_>) -> Result<()> {
        let started_at = Utc::now();
        self.started_at = Some(started_at);

        let env: BTreeMap<&String, &String> = invocation.env.iter().collect();
        let preloads: BTreeMap<&String, String> = invocation
            .preloads
            .iter()
            .map(|(name, path)| (name, path.display().to_string()))
            .collect();

        let predicate = json!({
            "runId": self.run_id,
            "startedAt": rfc3339(&started_at),
            "runtime": {
                "name": env!("CARGO_PKG_NAME"),
                "version": env!("CARGO_PKG_VERSION"),
                "engine": "lind-boot",
            },
            "keep": self.keep,
            "invocation": {
                "argv": invocation.argv,
                "env": env,
                "preloads": preloads,
            },
            "signer": self.signer(),
        });

        let envelope = self.envelope(PREDICATE_RUN_START, predicate)?;
        self.start_sha256 = Some(hex::encode(Sha256::digest(&envelope)));

        // The verifier needs the public key; write it next to the attestations.
        let public = serde_json::to_vec_pretty(self.key.public())
            .context("failed to serialize in-toto public key")?;
        self.emit("signer", "pub.json", &public, false);
        self.emit("run-start", "intoto.json", &envelope, self.echo);
        Ok(())
    }

    /// Emits the `run-end` attestation. Call right after lind-wasm returns.
    pub fn end(&mut self, outcome: Outcome<'_>) -> Result<()> {
        let started_at = self
            .started_at
            .ok_or_else(|| anyhow!("run-end attestation requested before run-start"))?;
        let finished_at = Utc::now();
        let duration_ms = (finished_at - started_at).num_milliseconds();

        let outcome = match outcome {
            Outcome::Exited(code) => json!({
                "status": "exited",
                "exitCode": code,
                "error": null,
            }),
            Outcome::Failed(e) => json!({
                "status": "failed",
                "exitCode": null,
                "error": format!("{e:#}"),
            }),
        };

        let predicate = json!({
            "runId": self.run_id,
            "startedAt": rfc3339(&started_at),
            "finishedAt": rfc3339(&finished_at),
            "durationMs": duration_ms,
            "outcome": outcome,
            "startAttestation": { "sha256": self.start_sha256 },
            "signer": self.signer(),
        });

        let envelope = self.envelope(PREDICATE_RUN_END, predicate)?;
        self.emit("run-end", "intoto.json", &envelope, self.echo);
        Ok(())
    }

    /// Description of the signing key, as recorded in the predicates.
    fn signer(&self) -> Value {
        json!({
            "keyid": json_string(self.key.key_id()),
            "keytype": json_string(self.key.public().typ()),
            "scheme": json_string(self.key.public().scheme()),
            "isKeepIdentityKey": self.signer_is_identity_key,
        })
    }

    /// Builds the statement, signs it and returns the serialized DSSE envelope.
    fn envelope(&self, predicate_type: &str, predicate: Value) -> Result<Vec<u8>> {
        let statement = json!({
            "_type": STATEMENT_TYPE,
            "subject": [self.subject],
            "predicateType": predicate_type,
            "predicate": predicate,
        });
        let payload = serde_json::to_vec(&statement).context("failed to serialize statement")?;

        let signature = self
            .key
            .sign(&pae(PAYLOAD_TYPE, &payload))
            .map_err(|e| anyhow!("failed to sign {predicate_type} attestation: {e:?}"))?;

        let envelope = json!({
            "payloadType": PAYLOAD_TYPE,
            "payload": BASE64.encode(&payload),
            "signatures": [{
                "keyid": json_string(signature.key_id()),
                "sig": BASE64.encode(signature.value().as_bytes()),
            }],
        });
        serde_json::to_vec_pretty(&envelope).context("failed to serialize envelope")
    }

    /// Writes `bytes` to `<dir>/lind-wasm-<kind>.<run id>.<ext>` and/or stderr.
    ///
    /// Never fails: attestations are too valuable to lose to a full disk, so a failed
    /// file write falls back to stderr.
    fn emit(&self, kind: &str, ext: &str, bytes: &[u8], echo: bool) {
        let name = format!("lind-wasm-{kind}.{}.{ext}", self.run_id);
        let mut to_stderr = echo;

        if let Sink::Dir(dir) = &self.sink {
            match write_file(dir, &name, bytes) {
                Ok(path) => eprintln!("[in-toto] wrote {}", path.display()),
                Err(e) => {
                    eprintln!("[in-toto] warning: {e:#}; printing {name} to stderr instead");
                    to_stderr = true;
                }
            }
        }

        if to_stderr {
            eprintln!("[in-toto] {name}");
            eprintln!("{}", String::from_utf8_lossy(bytes));
        }
    }
}

/// Loads the in-toto signing key.
///
/// The keep identity key is used directly when it is a P-256 key (SGX and KVM keeps),
/// the only ECDSA curve the `in-toto` crate signs with. Otherwise (SNP keeps use
/// P-384) an ephemeral Ed25519 key signs and the predicate binds it to the identity
/// key via `keep.identityKey`. The boolean says which case applies.
fn signer(identity_key: &[u8]) -> Result<(PrivateKey, bool)> {
    let pki =
        PrivateKeyInfo::from_der(identity_key).context("failed to parse keep identity key")?;
    let (_, curve) = pki
        .algorithm
        .oids()
        .context("failed to read keep identity key algorithm")?;

    if curve == Some(SECP_256_R_1) {
        let key = PrivateKey::from_pkcs8(identity_key, SignatureScheme::EcdsaP256Sha256)
            .map_err(|e| anyhow!("failed to load keep identity key for in-toto: {e:?}"))?;
        return Ok((key, true));
    }

    let der = PrivateKey::new(KeyType::Ed25519)
        .map_err(|e| anyhow!("failed to generate ephemeral in-toto signing key: {e:?}"))?;
    let key = PrivateKey::from_pkcs8(&der, SignatureScheme::Ed25519)
        .map_err(|e| anyhow!("failed to load ephemeral in-toto signing key: {e:?}"))?;
    Ok((key, false))
}

/// DSSE v1 pre-authentication encoding: the bytes that are actually signed.
pub fn pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "DSSEv1 {} {} {} ",
        payload_type.len(),
        payload_type,
        payload.len()
    )
    .into_bytes();
    out.extend_from_slice(payload);
    out
}

fn rfc3339(t: &DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The string a value serializes to (key ids, key types and schemes are all strings).
fn json_string<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn write_file(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    // `mkdir` may not be proxied out of the keep; a missing directory then shows up
    // as an error from the write below.
    let _ = fs::create_dir_all(dir);
    let path = dir.join(name);
    fs::write(&path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    use in_toto::crypto::{PublicKey, Signature};
    use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};

    fn p256_pkcs8() -> Vec<u8> {
        EcdsaKeyPair::generate_pkcs8(
            &ECDSA_P256_SHA256_ASN1_SIGNING,
            &ring::rand::SystemRandom::new(),
        )
        .unwrap()
        .as_ref()
        .to_vec()
    }

    fn cfg(dir: Option<PathBuf>) -> InToto {
        InToto {
            disabled: false,
            dir,
            stderr: false,
        }
    }

    fn keep<'a>(identity_key: &'a [u8], certs: &'a [Vec<u8>]) -> KeepInfo<'a> {
        KeepInfo {
            technology: "sgx",
            identity_key,
            identity_spki: b"spki",
            certificates: certs,
            steward: None,
        }
    }

    /// Verifies a DSSE envelope the way an external verifier would: decode, re-encode
    /// the PAE, check the signature with the published public key, return the statement.
    fn verify(envelope: &[u8], public: &PublicKey) -> Value {
        let env: Value = serde_json::from_slice(envelope).unwrap();
        assert_eq!(env["payloadType"], PAYLOAD_TYPE);
        let payload = BASE64
            .decode(env["payload"].as_str().unwrap().as_bytes())
            .unwrap();
        let sigs = env["signatures"].as_array().unwrap();
        assert_eq!(sigs.len(), 1);
        let sig = BASE64
            .decode(sigs[0]["sig"].as_str().unwrap().as_bytes())
            .unwrap();
        assert_eq!(sigs[0]["keyid"], json_string(public.key_id()));

        let signature: Signature = serde_json::from_value(json!({
            "keyid": sigs[0]["keyid"],
            "sig": hex::encode(sig),
        }))
        .unwrap();
        public
            .verify(&pae(PAYLOAD_TYPE, &payload), &signature)
            .unwrap();

        let statement: Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(statement["_type"], STATEMENT_TYPE);
        statement
    }

    #[test]
    fn disabled() {
        let key = p256_pkcs8();
        let cfg = InToto {
            disabled: true,
            ..cfg(None)
        };
        assert!(Attestor::new(&cfg, keep(&key, &[]), b"wasm")
            .unwrap()
            .is_none());
    }

    #[test]
    fn start_and_end_with_identity_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = p256_pkcs8();
        let certs = vec![b"cert".to_vec()];
        let wasm = b"\0asm\x01\0\0\0";

        let mut a = Attestor::new(&cfg(Some(dir.path().into())), keep(&key, &certs), wasm)
            .unwrap()
            .unwrap();
        assert!(a.signer_is_identity_key);
        let run_id = a.run_id.clone();

        let mut env = HashMap::new();
        env.insert("PRELOADS".to_string(), "/hello.cwasm".to_string());
        let argv = vec!["main.wasm".to_string(), "--log".to_string()];
        a.start(&Invocation {
            argv: &argv,
            env: &env,
            preloads: &[],
        })
        .unwrap();
        a.end(Outcome::Exited(3)).unwrap();

        let read = |kind: &str, ext: &str| {
            fs::read(dir.path().join(format!("lind-wasm-{kind}.{run_id}.{ext}"))).unwrap()
        };
        let public: PublicKey = serde_json::from_slice(&read("signer", "pub.json")).unwrap();
        assert_eq!(json_string(public.scheme()), "ecdsa-sha2-nistp256");

        let start_bytes = read("run-start", "intoto.json");
        let start = verify(&start_bytes, &public);
        assert_eq!(start["predicateType"], PREDICATE_RUN_START);
        assert_eq!(start["subject"][0]["name"], SUBJECT_NAME);
        assert_eq!(
            start["subject"][0]["digest"]["sha256"],
            hex::encode(Sha256::digest(wasm))
        );
        let p = &start["predicate"];
        assert_eq!(p["runId"], run_id);
        assert_eq!(p["invocation"]["argv"], json!(argv));
        assert_eq!(p["invocation"]["env"]["PRELOADS"], "/hello.cwasm");
        assert_eq!(p["keep"]["technology"], "sgx");
        assert_eq!(
            p["keep"]["certificates"][0]["sha256"],
            hex::encode(Sha256::digest(b"cert"))
        );
        assert_eq!(p["signer"]["isKeepIdentityKey"], true);
        assert_eq!(p["signer"]["keyid"], json_string(public.key_id()));

        let end = verify(&read("run-end", "intoto.json"), &public);
        assert_eq!(end["predicateType"], PREDICATE_RUN_END);
        let p = &end["predicate"];
        assert_eq!(p["runId"], run_id);
        assert_eq!(p["outcome"]["status"], "exited");
        assert_eq!(p["outcome"]["exitCode"], 3);
        assert_eq!(
            p["startAttestation"]["sha256"],
            hex::encode(Sha256::digest(&start_bytes))
        );
        assert!(p["durationMs"].as_i64().unwrap() >= 0);
    }

    #[test]
    fn failed_run_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let key = p256_pkcs8();
        let mut a = Attestor::new(&cfg(Some(dir.path().into())), keep(&key, &[]), b"wasm")
            .unwrap()
            .unwrap();
        a.start(&Invocation {
            argv: &[],
            env: &HashMap::new(),
            preloads: &[],
        })
        .unwrap();
        let err = anyhow!("trap").context("failed to run");
        a.end(Outcome::Failed(&err)).unwrap();

        let public: PublicKey = serde_json::from_slice(
            &fs::read(
                dir.path()
                    .join(format!("lind-wasm-signer.{}.pub.json", a.run_id)),
            )
            .unwrap(),
        )
        .unwrap();
        let end = verify(
            &fs::read(
                dir.path()
                    .join(format!("lind-wasm-run-end.{}.intoto.json", a.run_id)),
            )
            .unwrap(),
            &public,
        );
        assert_eq!(end["predicate"]["outcome"]["status"], "failed");
        assert_eq!(end["predicate"]["outcome"]["exitCode"], Value::Null);
        assert_eq!(end["predicate"]["outcome"]["error"], "failed to run: trap");
    }

    #[test]
    fn end_before_start_is_an_error() {
        let key = p256_pkcs8();
        let mut a = Attestor::new(&cfg(None), keep(&key, &[]), b"wasm")
            .unwrap()
            .unwrap();
        assert!(a.end(Outcome::Exited(0)).is_err());
    }

    #[test]
    fn non_p256_identity_key_uses_ephemeral_signer() {
        use ring::signature::ECDSA_P384_SHA384_ASN1_SIGNING;
        let key = EcdsaKeyPair::generate_pkcs8(
            &ECDSA_P384_SHA384_ASN1_SIGNING,
            &ring::rand::SystemRandom::new(),
        )
        .unwrap()
        .as_ref()
        .to_vec();
        let dir = tempfile::tempdir().unwrap();
        let mut a = Attestor::new(&cfg(Some(dir.path().into())), keep(&key, &[]), b"wasm")
            .unwrap()
            .unwrap();
        assert!(!a.signer_is_identity_key);
        a.start(&Invocation {
            argv: &[],
            env: &HashMap::new(),
            preloads: &[],
        })
        .unwrap();
        let public: PublicKey = serde_json::from_slice(
            &fs::read(
                dir.path()
                    .join(format!("lind-wasm-signer.{}.pub.json", a.run_id)),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(json_string(public.scheme()), "ed25519");
        let start = verify(
            &fs::read(
                dir.path()
                    .join(format!("lind-wasm-run-start.{}.intoto.json", a.run_id)),
            )
            .unwrap(),
            &public,
        );
        assert_eq!(start["predicate"]["signer"]["isKeepIdentityKey"], false);
    }

    #[test]
    fn pae_matches_dsse_spec() {
        assert_eq!(
            pae("http://example.com/HelloWorld", b"hello world"),
            b"DSSEv1 29 http://example.com/HelloWorld 11 hello world"
        );
    }
}
