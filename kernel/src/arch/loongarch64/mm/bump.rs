use crate::mm::{allocator::bump::BumpAllocator, MemoryManagementArch, PhysMemoryArea};

impl<MMA: MemoryManagementArch> BumpAllocator<MMA> {
    pub unsafe fn arch_remain_areas(_ret_areas: &mut [PhysMemoryArea], res_count: usize) -> usize {
        todo!("la64: arch_remain_areas")
    }
}
