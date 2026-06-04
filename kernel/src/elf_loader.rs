use x86_64::VirtAddr;
use x86_64::structures::paging::{
    FrameAllocator, Mapper, OffsetPageTable, Page, PageTableFlags, PhysFrame, Size4KiB, Translate,
};
use xmas_elf::ElfFile;
use xmas_elf::program::Type;

use crate::memory::BootInfoFrameAllocator;

pub fn load_elf(
    elf_bytes: &[u8],
    mapper: &mut OffsetPageTable,
    frame_allocator: &mut BootInfoFrameAllocator,
) -> u64 {
    let elf = ElfFile::new(elf_bytes).expect("failed to parse ELF");

    for header in elf.program_iter() {
        if header.get_type().unwrap() != Type::Load {
            continue;
        }

        let virt_start = header.virtual_addr();
        let mem_size = header.mem_size();
        let file_offset = header.offset() as usize;
        let file_size = header.file_size() as usize;

        let flags =
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;

        let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt_start));
        let end_page =
            Page::<Size4KiB>::containing_address(VirtAddr::new(virt_start + mem_size - 1));

        for page in Page::range_inclusive(start_page, end_page) {
            if mapper.translate_page(page).is_ok() {
                continue;
            }

            let frame = frame_allocator.allocate_frame().expect("out of frames");

            unsafe {
                mapper
                    .map_to(page, frame, flags, frame_allocator)
                    .expect("map_to failed")
                    .flush();
            }

            let page_ptr = page.start_address().as_mut_ptr::<u8>();
            unsafe {
                core::ptr::write_bytes(page_ptr, 0, 4096);
            }
        }

        let dest = virt_start as *mut u8;
        let src = &elf_bytes[file_offset..file_offset + file_size];
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), dest, file_size);
        }
    }

    elf.header.pt2.entry_point()
}
