use lazy_static::lazy_static;
use x86_64::VirtAddr;
use x86_64::instructions::segmentation::{CS, Segment};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

static mut TSS: TaskStateSegment = TaskStateSegment::new();

lazy_static! {
    static ref GDT: (GlobalDescriptorTable, Selectors) = {
        let mut gdt = GlobalDescriptorTable::new();
        let code_selector = gdt.add_entry(Descriptor::kernel_code_segment());
        let data_selector = gdt.add_entry(Descriptor::kernel_data_segment());
        let user_data_selector = gdt.add_entry(Descriptor::user_data_segment());
        let user_code_selector = gdt.add_entry(Descriptor::user_code_segment());
        let tss_selector = gdt.add_entry(Descriptor::tss_segment(unsafe {
            &*core::ptr::addr_of!(TSS)
        }));
        (
            gdt,
            Selectors {
                code_selector,
                data_selector,
                user_code_selector,
                user_data_selector,
                tss_selector,
            },
        )
    };
}

struct Selectors {
    code_selector: SegmentSelector,
    data_selector: SegmentSelector,
    user_data_selector: SegmentSelector,
    user_code_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

pub fn init() {
    unsafe {
        TSS.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            const STACK_SIZE: usize = 4096 * 5;
            static mut DF_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
            let stack_start = VirtAddr::from_ptr(core::ptr::addr_of!(DF_STACK));
            stack_start + STACK_SIZE as u64
        };
        TSS.privilege_stack_table[0] = {
            const STACK_SIZE: usize = 4096 * 5;
            static mut BOOT_STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
            let stack_start = VirtAddr::from_ptr(core::ptr::addr_of!(BOOT_STACK));
            stack_start + STACK_SIZE as u64
        };
    }

    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1.code_selector);
        load_tss(GDT.1.tss_selector);
    }
}

pub fn get_user_code_selector() -> SegmentSelector {
    GDT.1.user_code_selector
}

pub fn get_user_data_selector() -> SegmentSelector {
    GDT.1.user_data_selector
}

pub fn get_kernel_code_selector() -> SegmentSelector {
    GDT.1.code_selector
}

pub fn get_kernel_data_selector() -> SegmentSelector {
    GDT.1.data_selector
}

pub fn set_kernel_rsp0(stack_top: VirtAddr) {
    unsafe {
        TSS.privilege_stack_table[0] = stack_top;
    }
}
