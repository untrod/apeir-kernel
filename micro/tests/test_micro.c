#include "nous_micro.h"

#include <assert.h>
#include <string.h>

static int32_t allow(const nous_nmp_frame_v1 *frame, void *context) {
    (void)frame;
    (void)context;
    return 1;
}

static int32_t execute(const nous_nmp_frame_v1 *frame, void *context) {
    int *count = (int *)context;
    (void)frame;
    (*count)++;
    return 0;
}

int main(void) {
    nous_micro_runtime_v1 runtime;
    nous_nmp_frame_v1 frame;
    int count = 0;
    memset(&frame, 0, sizeof(frame));
    frame.magic = NOUS_NMP_MAGIC;
    frame.version = NOUS_NMP_VERSION;
    frame.message_type = 1u;
    frame.sequence = 1u;
    frame.payload_length = 1u;
    frame.payload[0] = 42u;
    frame.checksum = nous_nmp_checksum_v1(&frame);
    assert(nous_micro_init_v1(&runtime, allow, execute, &count) == NOUS_MICRO_OK);
    assert(runtime.struct_size == sizeof(runtime));
    assert(runtime.abi_version == NOUS_MICRO_ABI_VERSION);
    assert(nous_micro_static_bytes_v1() == sizeof(runtime));
    assert(nous_nmp_frame_bytes_v1() == sizeof(frame));
    assert(sizeof(runtime) <= 4096u);
    assert(nous_micro_submit_v1(&runtime, &frame) == NOUS_MICRO_OK);
    assert(nous_micro_tick_v1(&runtime) == NOUS_MICRO_OK);
    assert(count == 1);
    return 0;
}
