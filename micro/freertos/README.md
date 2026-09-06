# FreeRTOS adapter boundary

The Foundation release reserves FreeRTOS integration at the portable `nous_micro_*_v1`
ABI. The runtime itself has no operating-system dependencies, dynamic allocation,
threads, sockets, or file access. A future FreeRTOS transport task may feed validated
NMP frames into `nous_micro_submit_v1`; it must not bypass the safety callback.
