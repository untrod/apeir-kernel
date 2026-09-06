#include "nous_kernel.h"
#include <assert.h>
#include <stddef.h>

_Static_assert(sizeof(nous_profile_capabilities_v1) == 20u, "ABI size changed");
_Static_assert(offsetof(nous_profile_capabilities_v1, struct_size) == 0u, "ABI layout changed");
_Static_assert(offsetof(nous_profile_capabilities_v1, abi_version) == 4u, "ABI layout changed");
_Static_assert(offsetof(nous_profile_capabilities_v1, dynamic_allocation) == 12u, "ABI layout changed");

int main(void) {
    struct nous_profile_capabilities_v1 capabilities = {0};
    capabilities.struct_size = sizeof(capabilities);
    capabilities.abi_version = NOUS_KERNEL_ABI_VERSION;
    assert(nous_contract_version_v1(NOUS_CONTRACT_NEC) == 1u);
    assert(nous_contract_version_v1(NOUS_CONTRACT_NSE) == 1u);
    assert(nous_contract_version_v1(99u) == 0u);
    assert(nous_get_profile_capabilities_v1(NOUS_PROFILE_CORE, &capabilities) == 0);
    assert(capabilities.durable_journal == 1u);
    assert(capabilities.distributed_scheduling == 1u);
    assert(capabilities.transactional_effects == 1u);
    assert(nous_get_profile_capabilities_v1(99u, &capabilities) == -1);
    assert(nous_get_profile_capabilities_v1(NOUS_PROFILE_CORE, 0) == -1);
    capabilities.abi_version = 99u;
    assert(nous_get_profile_capabilities_v1(NOUS_PROFILE_CORE, &capabilities) == -2);
    return 0;
}
