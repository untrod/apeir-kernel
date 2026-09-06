#include "nous_kernel.h"
#include <string.h>

uint16_t nous_contract_version_v1(uint32_t contract) {
    return contract <= (uint32_t)NOUS_CONTRACT_NSE ? 1u : 0u;
}

int32_t nous_get_profile_capabilities_v1(
    uint32_t profile,
    struct nous_profile_capabilities_v1 *output) {
    if (output == 0 || profile > (uint32_t)NOUS_PROFILE_MICRO) {
        return -1;
    }
    if (output->struct_size < sizeof(*output)
        || output->abi_version != NOUS_KERNEL_ABI_VERSION) {
        return -2;
    }

    output->durable_journal = profile != (uint32_t)NOUS_PROFILE_MICRO;
    output->distributed_scheduling = profile == (uint32_t)NOUS_PROFILE_CORE;
    output->transactional_effects = 1u;
    output->governed_learning = profile == (uint32_t)NOUS_PROFILE_CORE;
    output->dynamic_allocation = profile != (uint32_t)NOUS_PROFILE_MICRO;
    memset(output->reserved, 0, sizeof(output->reserved));
    return 0;
}
