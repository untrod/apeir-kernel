#include "nous_micro.h"

#include <string.h>

uint32_t nous_nmp_checksum_v1(const nous_nmp_frame_v1 *frame) {
    uint32_t hash = 2166136261u;
    uint16_t index;
    if (frame == NULL || frame->payload_length > NOUS_MICRO_PAYLOAD_CAPACITY) {
        return 0u;
    }
    hash = (hash ^ frame->message_type) * 16777619u;
    hash = (hash ^ frame->flags) * 16777619u;
    hash = (hash ^ (uint32_t)frame->sequence) * 16777619u;
    hash = (hash ^ (uint32_t)(frame->sequence >> 32u)) * 16777619u;
    for (index = 0u; index < frame->payload_length; ++index) {
        hash = (hash ^ frame->payload[index]) * 16777619u;
    }
    return hash;
}

int32_t nous_micro_init_v1(
    nous_micro_runtime_v1 *runtime,
    nous_micro_safety_gate_v1 safety_gate,
    nous_micro_effector_v1 effector,
    void *context) {
    if (runtime == NULL || safety_gate == NULL || effector == NULL) {
        return NOUS_MICRO_INVALID;
    }
    memset(runtime, 0, sizeof(*runtime));
    runtime->struct_size = (uint32_t)sizeof(*runtime);
    runtime->abi_version = NOUS_MICRO_ABI_VERSION;
    runtime->safety_gate = safety_gate;
    runtime->effector = effector;
    runtime->context = context;
    return NOUS_MICRO_OK;
}

int32_t nous_micro_submit_v1(
    nous_micro_runtime_v1 *runtime,
    const nous_nmp_frame_v1 *frame) {
    if (runtime == NULL || frame == NULL
        || runtime->struct_size != sizeof(*runtime)
        || runtime->abi_version != NOUS_MICRO_ABI_VERSION
        || frame->magic != NOUS_NMP_MAGIC
        || frame->version != NOUS_NMP_VERSION
        || frame->payload_length > NOUS_MICRO_PAYLOAD_CAPACITY
        || frame->checksum != nous_nmp_checksum_v1(frame)) {
        return NOUS_MICRO_INVALID;
    }
    if (runtime->count == NOUS_MICRO_QUEUE_CAPACITY) {
        return NOUS_MICRO_QUEUE_FULL;
    }
    runtime->queue[runtime->tail] = *frame;
    runtime->tail = (uint8_t)((runtime->tail + 1u) % NOUS_MICRO_QUEUE_CAPACITY);
    runtime->count++;
    return NOUS_MICRO_OK;
}

int32_t nous_micro_tick_v1(nous_micro_runtime_v1 *runtime) {
    nous_nmp_frame_v1 *frame;
    int32_t result;
    if (runtime == NULL
        || runtime->struct_size != sizeof(*runtime)
        || runtime->abi_version != NOUS_MICRO_ABI_VERSION
        || runtime->count == 0u) {
        return NOUS_MICRO_INVALID;
    }
    frame = &runtime->queue[runtime->head];
    result = runtime->safety_gate(frame, runtime->context);
    if (result == 0) {
        result = NOUS_MICRO_SAFETY_DENIED;
    } else if (runtime->effector(frame, runtime->context) != 0) {
        result = NOUS_MICRO_EFFECT_FAILED;
    } else {
        result = NOUS_MICRO_OK;
    }
    runtime->head = (uint8_t)((runtime->head + 1u) % NOUS_MICRO_QUEUE_CAPACITY);
    runtime->count--;
    return result;
}

size_t nous_micro_static_bytes_v1(void) {
    return sizeof(nous_micro_runtime_v1);
}

size_t nous_nmp_frame_bytes_v1(void) {
    return sizeof(nous_nmp_frame_v1);
}
