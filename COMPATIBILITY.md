# Compatibility policy

Foundation contracts and Open Runtime schemas are versioned independently.
Within v1, additions must be optional and old documents must remain readable.
Removing a field, changing semantics, weakening safety, or changing durable
state interpretation requires a new major version and migration guidance.

NKI clients must negotiate the existing protocol version. Provider responses
are normalized before they cross the kernel boundary. Declarative manifests
reject unknown fields so misspellings do not silently alter execution.

Version `0.x` means the project API remains pre-release. Contracts explicitly
marked v1 are frozen according to the rules above; this does not make every
runtime capability production-ready.
