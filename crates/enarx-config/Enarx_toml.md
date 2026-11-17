# Configure Enarx.toml

With the `Enarx.toml` configuration file, environment variables, arguments and pre-opened file descriptors
can be passed to the WASM application.

## Elements

All elements are optional.

### `env`

`env` specifies the environment variables exported to the WASM application in a map.

#### Example

```toml
[env]
VAR1 = "var1"
VAR2 = "var2"
```

### `args`

`args` specifies the arguments for the WASM application in an array.

#### Example

```toml
args = [
     "--argument1",
     "--argument2=foo"
]
```

### `steward`

`steward` specifies the URL for the steward to contact for a TLS certificate.

#### Example

```toml
steward = "https://attest.profian.com"
```

### `intoto`

`intoto` configures the in-toto attestation of the run. Two attestations are produced per
run, one right before the lind-wasm runtime starts (`run-start`) and one right after it ends
(`run-end`). Each is an in-toto Statement v1 in a DSSE v1 envelope, signed with the keep
identity key, so it chains to the TEE attestation of the keep.

| key        | default | meaning                                                                 |
|------------|---------|-------------------------------------------------------------------------|
| `dir`      | unset   | Host directory the files are written to. Unset: printed to stderr.      |
| `stderr`   | `false` | Also print the attestations to stderr when `dir` is set.                |
| `disabled` | `false` | Do not produce attestations.                                            |

With `dir` set, every run `<id>` writes three files there:

* `lind-wasm-run-start.<id>.intoto.json`: subject `main.wasm` (sha256/sha512 of the module),
  predicate `https://lind-project.org/attestation/lind-wasm/run-start/v1` with the guest
  argv/env/preloads and clock epoch, the TEE technology, the keep identity key and
  certificate digests, and the signer.
* `lind-wasm-run-end.<id>.intoto.json`: same subject, predicate
  `https://lind-project.org/attestation/lind-wasm/run-end/v1` with the exit code or runtime
  error, the duration, and the sha256 of the `run-start` file.
* `lind-wasm-signer.<id>.pub.json`: the public key in in-toto (securesystemslib) format.

A workload that records its own evidence (the in-toto grate) gets `run-end` during the run
instead: its first cage hands in the key it signs with and the hash of the last statement of
its chain (lind's `LIND_EVIDENCE`), and `run-end`, which names both, goes back to it with
`run-start`, the identity key and the TEE attestation report, to be dumped with the build's
outputs. No `run-end` file is written then; once the build has started, a sealed keep lets
nothing out but its console and dump region.

#### Example

```toml
[intoto]
dir = "/var/lib/enarx/attestations"
```

### `clock`

`clock` declares the date the run sees. An SGX keep never asks the host for the time of
day: the real-time clock reads `epoch` (seconds since 1970-01-01T00:00:00Z, default 0)
when the workload starts and advances from there with the TSC, and the monotonic clock
(also standing in for the CPU-time clocks) starts at 0 when the keep starts. Neither ever
reads less than it did before, even if the host moves the TSC. `run-start` records the
epoch under `invocation.clock`, with the other inputs of the run.

#### Example

```toml
[clock]
epoch = 1760659200
```

### `files`

`files` specifies an array of file descriptor definitions to be pre-opened for the WASM application.

A `files` entry can contain the following sub elements.

#### `kind`

`kind` can be one of `"null"`, `"stdin"`,`"stdout"`, `"stderr"`, `"listen"` or `"connect"`.

#### `name`

Name of the file descriptor, exported in the `FD_NAMES` environment variable.
The default `name` for `kind`  `"null"`, `"stdin"`,`"stdout"`, `"stderr"` is the `kind`. 

The `FD_NAMES` environment variable contains all `name` strings of the `files` array joined with ":".
The `FD_COUNT` environment variable contains the number of `files` elements.

#### `prot`

`prot` can be `"tcp"` or `"tls"` for `kind = "connect"` or `kind = "listen"`.

`"tls"` is the default, if `prot` is not specified.

`tls` transparently wraps a TCP connection with the TLS protocol.
For `kind = "listen"` every accepted connection is also wrapped with the TLS protocol. 

#### `host`

`host` specifies the host to connect to for a `kind = "connect"`

#### `addr`

`addr` specifies the address to bind to for a `kind = "listen"`.

##### Examples

```toml
addr = "::"          # bind to any interface IPv6 and IPv4 (the default, if not specified)
addr = "0.0.0.0"     # bind to any IPv4 interface
addr = "::1"         # bind to IPv6 localhost
addr = "127.0.0.1"   # bind to IPv4 localhost
addr = "192.168.1.1" # bind to a specific IPv4 address
```

#### `port`

`port` specifies the port to connect or bind to for `kind = "connect"` or `kind = "listen"`.
The default value is `443`.

## Example
```toml
# Configuration for a WASI application in an Enarx Keep

# Arguments
args = [
     "--argument1",
     "--argument2=foo"
]

# Environment variables
[env]
VAR1 = "var1"
VAR2 = "var2"

# Pre-opened file descriptors
[[files]]
kind = "null"

[[files]]
kind = "stdout"

[[files]]
kind = "stderr"

# A listen socket
[[files]]
name = "listen"
kind = "listen"
prot = "tls" # or prot = "tcp"
port = 12345

# An outgoing connected socket
[[files]]
name = "connect"
kind = "connect"
prot = "tcp" # or prot = "tls"
host = "127.0.0.1"
port = 23456
```

This configuration files passes the environment `VAR1=var1 VAR2=var2` and the arguments `--argument1 --argument2=foo` to the WASM application.

Additionally, five file descriptors are pre-opened:
- 0: `/dev/null`
- 1: `/dev/stdout`
- 2: `/dev/stderr`
- 3: a TCP listen socket bound to port `12345` on address `::`, where every accepted connection is transparently wrapped with the TLS protocol 
- 4: a normal TCP stream socket connected to `127.0.0.1:23456`

Additionally, the following environment variables are exported:
- `FD_COUNT=5`
- `FD_NAMES=null:stdout:stderr:listen:connect`
