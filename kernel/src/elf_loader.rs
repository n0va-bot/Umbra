use x86_64::VirtAddr;
use x86_64::structures::paging::{
    FrameAllocator, Mapper, OffsetPageTable, Page, PageTableFlags, Size4KiB,
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

pub fn load_elf_into(
    elf_bytes: &[u8],
    mapper: &mut OffsetPageTable,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> u64 {
    let phys_mem_offset = crate::memory::get_phys_mem_offset();

    crate::serial_println!(
        "[load_elf_into] First 4 bytes: {:x?} (should be 7f 45 4c 46)",
        &elf_bytes[0..4]
    );
    let elf = ElfFile::new(elf_bytes).expect("failed to parse ELF");

    for header in elf.program_iter() {
        if header.get_type().unwrap() != Type::Load {
            continue;
        }

        let virt_start = header.virtual_addr();
        let mem_size = header.mem_size();
        let file_offset = header.offset() as usize;
        let file_size = header.file_size() as usize;

        crate::serial_println!(
            "[load_elf_into] virt_start: {:#x}, file_offset: {:#x}, file_size: {:#x}, mem_size: {:#x}",
            virt_start,
            file_offset,
            file_size,
            mem_size
        );

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

            let frame_virt = phys_mem_offset + frame.start_address().as_u64();
            unsafe {
                core::ptr::write_bytes(frame_virt.as_mut_ptr::<u8>(), 0, 4096);
            }
        }

        let mut remaining = file_size;
        let mut src_offset = file_offset;
        let mut virt_addr = virt_start;

        while remaining > 0 {
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt_addr));
            let frame = mapper
                .translate_page(page)
                .expect("load_elf_into: page should be mapped after map_to");
            let page_offset = (virt_addr & 0xFFF) as usize;
            let chunk_len = core::cmp::min(4096 - page_offset, remaining);

            let dest_virt = phys_mem_offset + frame.start_address().as_u64() + page_offset as u64;
            unsafe {
                core::ptr::copy_nonoverlapping(
                    elf_bytes[src_offset..].as_ptr(),
                    dest_virt.as_mut_ptr::<u8>(),
                    chunk_len,
                );
            }

            src_offset += chunk_len;
            virt_addr += chunk_len as u64;
            remaining -= chunk_len;
        }
    }

    elf.header.pt2.entry_point()
}
