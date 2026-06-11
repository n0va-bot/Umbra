use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{FrameAllocator, OffsetPageTable, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr, structures::paging::PageTable};

extern crate alloc;
use alloc::vec::Vec;

/// Global physical memory offset
static mut PHYS_MEM_OFFSET: u64 = 0;

pub fn store_phys_mem_offset(offset: VirtAddr) {
    unsafe { PHYS_MEM_OFFSET = offset.as_u64() };
}

pub fn get_phys_mem_offset() -> VirtAddr {
    unsafe { VirtAddr::new(PHYS_MEM_OFFSET) }
}

unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    let (level_4_table_frame, _) = Cr3::read();

    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    unsafe { &mut *page_table_ptr }
}

pub unsafe fn translate_addr(addr: VirtAddr, physical_memory_offset: VirtAddr) -> Option<PhysAddr> {
    translate_addr_inner(addr, physical_memory_offset)
}

fn translate_addr_inner(addr: VirtAddr, physical_memory_offset: VirtAddr) -> Option<PhysAddr> {
    use x86_64::structures::paging::page_table::FrameError;

    let (level_4_table_frame, _) = Cr3::read();

    let table_indexes = [
        addr.p4_index(),
        addr.p3_index(),
        addr.p2_index(),
        addr.p1_index(),
    ];
    let mut frame = level_4_table_frame;

    for &index in &table_indexes {
        let virt = physical_memory_offset + frame.start_address().as_u64();
        let table_ptr: *const PageTable = virt.as_ptr();
        let table = unsafe { &*table_ptr };
        let entry = &table[index];
        frame = match entry.frame() {
            Ok(frame) => frame,
            Err(FrameError::FrameNotPresent) => return None,
            Err(FrameError::HugeFrame) => panic!("huge pages not supported"),
        };
    }

    Some(frame.start_address() + u64::from(addr.page_offset()))
}

pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    unsafe {
        let level_4_table = active_level_4_table(physical_memory_offset);
        OffsetPageTable::new(level_4_table, physical_memory_offset)
    }
}

pub unsafe fn create_mapper_for_pml4(pml4_phys: PhysAddr) -> OffsetPageTable<'static> {
    let offset = get_phys_mem_offset();
    let pml4_virt = offset + pml4_phys.as_u64();
    let pml4 = unsafe { &mut *(pml4_virt.as_mut_ptr() as *mut PageTable) };
    unsafe { OffsetPageTable::new(pml4, offset) }
}

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

pub fn create_example_mapping(
    page: Page,
    mapper: &mut OffsetPageTable,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) {
    use x86_64::structures::paging::PageTableFlags as Flags;

    let frame = PhysFrame::containing_address(PhysAddr::new(0xb8000));
    let flags = Flags::PRESENT | Flags::WRITABLE;

    let map_to_result = unsafe {
        // FIXME: whatever the fuck this is
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

pub struct BootInfoFrameAllocator {
    memory_map: &'static mut MemoryRegions,
    next: usize,
    recycled: Vec<PhysFrame>,
}

impl BootInfoFrameAllocator {
    pub unsafe fn init(memory_map: &'static mut MemoryRegions) -> Self {
        BootInfoFrameAllocator {
            memory_map,
            next: 0,
            recycled: Vec::new(),
        }
    }

    pub fn deallocate_frame(&mut self, frame: PhysFrame) {
        let virt = get_phys_mem_offset() + frame.start_address().as_u64();
        unsafe {
            core::ptr::write_bytes(virt.as_mut_ptr::<u8>(), 0, 4096);
        }
        self.recycled.push(frame);
    }
}

use bootloader_api::info::MemoryRegionKind;

impl BootInfoFrameAllocator {
    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> {
        let regions = self.memory_map.iter();
        let usable_regions = regions.filter(|r| r.kind == MemoryRegionKind::Usable);
        let addr_ranges = usable_regions.map(|r| r.start..r.end);
        let frame_addresses = addr_ranges.flat_map(|r| r.step_by(4096));
        frame_addresses.map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        if let Some(frame) = self.recycled.pop() {
            return Some(frame);
        }
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}
