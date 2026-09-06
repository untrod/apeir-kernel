#ifndef NOUS_KERNEL_H
#define NOUS_KERNEL_H

#include <stdint.h>

#define NOUS_KERNEL_ABI_VERSION 1u

#if defined(_WIN32) && defined(NOUS_KERNEL_SHARED)
#if defined(NOUS_KERNEL_BUILD)
#define NOUS_KERNEL_API __declspec(dllexport)
#else
#define NOUS_KERNEL_API __declspec(dllimport)
#endif
#elif defined(__GNUC__) && defined(NOUS_KERNEL_SHARED)
#define NOUS_KERNEL_API __attribute__((visibility("default")))
#else
#define NOUS_KERNEL_API
#endif

#ifdef __cplusplus
extern "C" {
#endif

typedef enum nous_contract_v1 {
    NOUS_CONTRACT_NEC = 0,
    NOUS_CONTRACT_NSO = 1,
    NOUS_CONTRACT_NTE = 2,
    NOUS_CONTRACT_NRP = 3,
    NOUS_CONTRACT_NPA = 4,
    NOUS_CONTRACT_NKI = 5,
    NOUS_CONTRACT_NMP = 6,
    NOUS_CONTRACT_NCT = 7,
    NOUS_CONTRACT_NLC = 8,
    NOUS_CONTRACT_NSE = 9
} nous_contract_v1;

typedef enum nous_runtime_profile_v1 {
    NOUS_PROFILE_CORE = 0,
    NOUS_PROFILE_EDGE = 1,
    NOUS_PROFILE_MICRO = 2
} nous_runtime_profile_v1;

typedef struct nous_profile_capabilities_v1 {
    uint32_t struct_size;
    uint32_t abi_version;
    uint8_t durable_journal;
    uint8_t distributed_scheduling;
    uint8_t transactional_effects;
    uint8_t governed_learning;
    uint8_t dynamic_allocation;
    uint8_t reserved[7];
} nous_profile_capabilities_v1;

NOUS_KERNEL_API uint16_t nous_contract_version_v1(uint32_t contract);
NOUS_KERNEL_API int32_t nous_get_profile_capabilities_v1(
    uint32_t profile,
    struct nous_profile_capabilities_v1 *output);

#ifdef __cplusplus
}
#endif

#endif
