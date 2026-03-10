use crate::layout::{SHM_CLIENT_POOL_VA, SHM_VA};
use glenda::cap::CapPtr;
use glenda::client::ResourceClient;
use glenda::error::Error;
use glenda::interface::{CSpaceService, VSpaceService};
pub use glenda::mem::pool::{MemoryPool as ShmPool, ShmType};
pub use glenda::mem::shm::SharedMemory;

/// Central Memory Pool for managing SHM allocations in Prism.
/// Wraps two underlying generic Pools for different virtual address regions.
pub struct MemoryPool {
    dma_pool: ShmPool,
    client_pool: ShmPool,
}

impl MemoryPool {
    pub fn new() -> Self {
        Self { dma_pool: ShmPool::new(SHM_VA), client_pool: ShmPool::new(SHM_CLIENT_POOL_VA) }
    }

    /// Allocate a frame from the central resource manager and map it.
    pub fn alloc_shm(
        &mut self,
        vspace: &mut dyn VSpaceService,
        cspace: &mut dyn CSpaceService,
        res_client: &mut ResourceClient,
        size: usize,
        shm_type: ShmType,
        recv_slot: CapPtr,
    ) -> Result<SharedMemory, Error> {
        match shm_type {
            ShmType::DMA => {
                self.dma_pool.alloc_shm(vspace, cspace, res_client, size, shm_type, recv_slot)
            }
            ShmType::Regular => {
                self.client_pool.alloc_shm(vspace, cspace, res_client, size, shm_type, recv_slot)
            }
        }
    }

    pub fn reserve_vaddr_dma(&self, size: usize) -> usize {
        self.dma_pool.reserve(size)
    }

    pub fn get_dma_pool(&self) -> &[SharedMemory] {
        self.dma_pool.shms()
    }

    pub fn get_client_pool(&self) -> &[SharedMemory] {
        self.client_pool.shms()
    }
}
