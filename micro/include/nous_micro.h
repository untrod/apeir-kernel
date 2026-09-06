#ifndef NOUS_MICRO_H
#define NOUS_MICRO_H

#include <stddef.h>
#include <stdint.h>

#define NOUS_NMP_MAGIC 0x4e4d5031u
#define NOUS_NMP_VERSION 1u
#define NOUS_MICRO_ABI_VERSION 1u
#define NOUS_MICRO_QUEUE_CAPACITY 8u
#define NOUS_MICRO_PAYLOAD_CAPACITY 256u

typedef enum nous_micro_result_v1 {
    NOUS_MICRO_OK = 0,
    NOUS_MICRO_INVALID = -1,
    NOUS_MICRO_QUEUE_FULL = -2,
    NOUS_MICRO_SAFETY_DENIED = -3,
    NOUS_MICRO_EFFECT_FAILED = -4
} nous_micro_result_v1;

typedef struct nous_nmp_frame_v1 {
    uint32_t magic;
    uint16_t version;
    uint16_t message_type;
    uint32_t flags;
    uint64_t sequence;
    uint16_t payload_length;
    uint8_t payload[NOUS_MICRO_PAYLOAD_CAPACITY];
    uint32_t checksum;
} nous_nmp_frame_v1;

typedef int32_t (*nous_micro_safety_gate_v1)(const nous_nmp_frame_v1 *frame, void *context);
typedef int32_t (*nous_micro_effector_v1)(const nous_nmp_frame_v1 *frame, void *context);

typedef struct nous_micro_runtime_v1 {
    uint32_t struct_size;
    uint32_t abi_version;
    nous_nmp_frame_v1 queue[NOUS_MICRO_QUEUE_CAPACITY];
    uint8_t head;
    uint8_t tail;
    uint8_t count;
    nous_micro_safety_gate_v1 safety_gate;
    nous_micro_effector_v1 effector;
    void *context;
} nous_micro_runtime_v1;

uint32_t nous_nmp_checksum_v1(const nous_nmp_frame_v1 *frame);
int32_t nous_micro_init_v1(
    nous_micro_runtime_v1 *runtime,
    nous_micro_safety_gate_v1 safety_gate,
    nous_micro_effector_v1 effector,
    void *context);
int32_t nous_micro_submit_v1(
    nous_micro_runtime_v1 *runtime,
    const nous_nmp_frame_v1 *frame);
int32_t nous_micro_tick_v1(nous_micro_runtime_v1 *runtime);
size_t nous_micro_static_bytes_v1(void);
size_t nous_nmp_frame_bytes_v1(void);

#endif
