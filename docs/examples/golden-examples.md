# Golden examples

## Deterministic mathematics

`examples/open-runtime/mathematics/model.yaml` binds
`mathematics.evaluate` to the native `reference-math` backend. Linear and
polynomial inputs have exact expected results and require no network service.

```text
apeir-kernelctl run --input-file examples/open-runtime/mathematics/input.json --backend reference-math
```

This is the recommended first execution example because it exercises the
normal kernel path without credentials or external availability.

## OpenAI-compatible declaration

`examples/open-runtime/reasoning/model.yaml` demonstrates a remote model
declaration. Execution requires an operator-configured HTTPS endpoint and the
name of an allowlisted credential environment variable. The example contains
no key and does not certify a particular model vendor or account.

## Vision declaration

`examples/open-runtime/vision/model.yaml` demonstrates an ONNX vision contract.
The repository does not distribute weights or datasets. Schema validation does
not establish that a model is licensed, compatible, accurate, or executable on
a particular accelerator; those properties require integration-specific tests.
