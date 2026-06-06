use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{FrameAllocator, OffsetPageTable, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr, structures::paging::PageTable};

/// Global physical memory offset
static mut PHYS_MEM_OFFSET: u64 = 0;

/// Store the physical memory offset
pub fn store_phys_mem_offset(offset: VirtAddr) {
    unsafe { PHYS_MEM_OFFSET = offset.as_u64() };
}

/// Retrieve the stored physical memory offset
pub fn get_phys_mem_offset() -> VirtAddr {
    unsafe { VirtAddr::new(PHYS_MEM_OFFSET) }
}

/// Returns a mutable reference to the active level 4 table.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`. Also, this function must be only called once
/// to avoid aliasing `&mut` references (which is undefined behavior).
unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    let (level_4_table_frame, _) = Cr3::read();

    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    unsafe { &mut *page_table_ptr }
}

/// Translates the given virtual address to the mapped physical address, or
/// `None` if the address is not mapped.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`.
pub unsafe fn translate_addr(addr: VirtAddr, physical_memory_offset: VirtAddr) -> Option<PhysAddr> {
    translate_addr_inner(addr, physical_memory_offset)
}

/// Private function that is called by `translate_addr`.
///
/// This function is safe to limit the scope of `unsafe` because Rust treats
/// the whole body of unsafe functions as an unsafe block. This function must
/// only be reachable through `unsafe fn` from outside of this module.
fn translate_addr_inner(addr: VirtAddr, physical_memory_offset: VirtAddr) -> Option<PhysAddr> {
    use x86_64::structures::paging::page_table::FrameError;

    // read the active level 4 frame from the CR3 register
    let (level_4_table_frame, _) = Cr3::read();

    let table_indexes = [
        addr.p4_index(),
        addr.p3_index(),
        addr.p2_index(),
        addr.p1_index(),
    ];
    let mut frame = level_4_table_frame;

    // traverse the multi-level page table
    for &index in &table_indexes {
        // convert the frame into a page table reference
        let virt = physical_memory_offset + frame.start_address().as_u64();
        let table_ptr: *const PageTable = virt.as_ptr();
        let table = unsafe { &*table_ptr };

        // read the page table entry and update `frame`
        let entry = &table[index];
        frame = match entry.frame() {
            Ok(frame) => frame,
            Err(FrameError::FrameNotPresent) => return None,
            Err(FrameError::HugeFrame) => panic!("huge pages not supported"),
        };
    }

    // calculate the physical address by adding the page offset
    Some(frame.start_address() + u64::from(addr.page_offset()))
}

/// Initialize a new OffsetPageTable for the boot (current) CR3.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`. Also, this function must be only called once
/// to avoid aliasing `&mut` references (which is undefined behavior).
pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    unsafe {
        let level_4_table = active_level_4_table(physical_memory_offset);
        OffsetPageTable::new(level_4_table, physical_memory_offset)
    }
}

/// Create an `OffsetPageTable` for an **arbitrary** PML4 whose physical
/// address is known.  Used by `process::spawn` to set up mappings in a
/// brand-new address space without switching CR3 first.
///
/// # Safety
///
/// The caller must ensure `pml4_phys` points to a valid PML4 frame and that
/// the physical memory offset is valid.
pub unsafe fn create_mapper_for_pml4(pml4_phys: PhysAddr) -> OffsetPageTable<'static> {
    let offset = get_phys_mem_offset();
    let pml4_virt = offset + pml4_phys.as_u64();
    let pml4 = unsafe { &mut *(pml4_virt.as_mut_ptr() as *mut PageTable) };
    unsafe { OffsetPageTable::new(pml4, offset) }
}

/// Clone **all kernel mappings** from the current PML4 into a brand-new PML4
/// frame, leaving user-space entries (the ones that will hold ELF code /
/// stack) zeroed.
///
/// This copies every non-empty PML4 entry from the boot page tables.  That
/// includes:
///   - higher-half entries 256-511 (kernel code, physical-memory mapping, …)
///   - lower-half entries like the heap at 0x4444_4444_0000
///
/// The new PML4 shares the same PDP/PD/PT frames as the boot PML4 for all
/// kernel regions (copy-on-write would be an optimisation, but for a hobby OS
/// with < 16 processes this is fine).
///
/// Returns the **physical** address of the new PML4 frame (suitable for CR3).
///
/// # Safety
///
/// Caller must ensure the frame allocator returns unused frames.
pub unsafe fn clone_kernel_pml4(frame_allocator: &mut impl FrameAllocator<Size4KiB>) -> PhysAddr {
    let offset = get_phys_mem_offset();
    let (current_pml4_frame, _) = Cr3::read();
    let current_pml4_ptr =
        (offset + current_pml4_frame.start_address().as_u64()).as_ptr() as *const u64;

    let new_pml4_frame = frame_allocator
        .allocate_frame()
        .expect("out of frames for PML4");
    let new_pml4_ptr = (offset + new_pml4_frame.start_address().as_u64()).as_mut_ptr() as *mut u64;

    for i in 0..512 {
        unsafe { *new_pml4_ptr.add(i) = 0 };
    }

    for i in 0..512 {
        let entry_bits = unsafe { *current_pml4_ptr.add(i) };
        if entry_bits != 0 {
            unsafe { *new_pml4_ptr.add(i) = entry_bits };
        }
    }

    new_pml4_frame.start_address()
}

use x86_64::structures::paging::{Mapper, Page};

/// Creates an example mapping for the given page to frame `0xb8000`.
pub fn create_example_mapping(
    page: Page,
    mapper: &mut OffsetPageTable,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) {
    use x86_64::structures::paging::PageTableFlags as Flags;

    let frame = PhysFrame::containing_address(PhysAddr::new(0xb8000));
    let flags = Flags::PRESENT | Flags::WRITABLE;

    let map_to_result = unsafe {
        // FIXME: this is not safe, we do it only for testing
        mapper.map_to(page, frame, flags, frame_allocator)
    };
    map_to_result.expect("map_to failed").flush();
}

pub struct EmptyFrameAllocator;

unsafe impl FrameAllocator<Size4KiB> for EmptyFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        None
    }
}

use bootloader_api::info::MemoryRegions;

/// A FrameAllocator that returns usable frames from the bootloader's memory map.
pub struct BootInfoFrameAllocator {
    memory_map: &'static mut MemoryRegions,
    next: usize,
}

impl BootInfoFrameAllocator {
    /// Create a FrameAllocator from the passed memory map.
    ///
    /// This function is unsafe because the caller must guarantee that the passed
    /// memory map is valid. The main requirement is that all frames that are marked
    /// as `USABLE` in it are really unused.
    pub unsafe fn init(memory_map: &'static mut MemoryRegions) -> Self {
        BootInfoFrameAllocator {
            memory_map,
            next: 0,
        }
    }
}

use bootloader_api::info::MemoryRegionKind;

impl BootInfoFrameAllocator {
    /// Returns an iterator over the usable frames specified in the memory map.
    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> {
        // get usable regions from memory map
        let regions = self.memory_map.iter();
        let usable_regions = regions.filter(|r| r.kind == MemoryRegionKind::Usable);
        // map each region to its address range
        let addr_ranges = usable_regions.map(|r| r.start..r.end);
        // transform to an iterator of frame start addresses
        let frame_addresses = addr_ranges.flat_map(|r| r.step_by(4096));
        // create `PhysFrame` types from the start addresses
        frame_addresses.map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}
