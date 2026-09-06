# Extension security boundary

Provider manifests are untrusted input. They cannot contain credential values.
The operational configuration names an allow-listed environment variable, and
the provider process receives a cleared environment plus the minimum runtime
variables. External entrypoints are launched without a shell. Parent traversal
is rejected. Timeout and cancellation kill the directly hosted provider
process, and malformed output is converted to a standard Provider error.
