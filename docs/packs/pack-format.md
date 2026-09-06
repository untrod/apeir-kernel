# Pack format v1

A Pack groups references to models, providers, workflows, policies, and
resources. It is an application-level composition format, not a package manager
and not a kernel subsystem.

`apeir-kernelctl new project <name>` creates a portable directory with `nous.yaml` and the
standard model, provider, workflow, policy, resource, and test directories.
`apeir-kernelctl pack validate nous.yaml` validates the manifest. Applying or downloading
packs is deliberately outside v1.
