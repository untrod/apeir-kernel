# Device contract

DeviceSpec declares architecture, topology, resources, and device capabilities.
Devices remain untrusted extensions and must expose operations through Provider
and Capability contracts. Physical effects require safety and transactional
effect rules above the adapter. Current device support is type-level and Micro C
host evidence; no physical device is supported by this release candidate.
