# Micro runtime

APEIR Micro is a caller-owned, fixed-capacity C runtime. It contains eight NMP
frames, queue indexes, two callbacks, and one opaque context pointer. The
implementation calls no allocation API and has no background thread.

`nous_micro_static_bytes_v1()` and `nous_nmp_frame_bytes_v1()` expose the exact
ABI sizes for the target compiler. The host conformance test asserts that the
complete runtime remains at or below 4 KiB. Stack usage is bounded by local
pointers, integers, and callback frames; payload storage is static inside the
runtime object.

The portable host implementation is verified. Zephyr and FreeRTOS scheduling,
transport, signed configuration, watchdog, and target flash measurements are
separate platform work and remain unverified.
