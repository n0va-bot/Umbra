use acpi::{AcpiHandler, AcpiTables, PhysicalMapping, fadt::Fadt};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::port::Port;

static PHYSICAL_MEMORY_OFFSET: AtomicU64 = AtomicU64::new(0);

pub fn init(offset: u64) {
    PHYSICAL_MEMORY_OFFSET.store(offset, Ordering::SeqCst);
}

#[derive(Clone)]
pub struct UmbraAcpiHandler {
    physical_memory_offset: u64,
}

impl UmbraAcpiHandler {
    pub fn new(physical_memory_offset: u64) -> Self {
        Self {
            physical_memory_offset,
        }
    }
}

impl AcpiHandler for UmbraAcpiHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> PhysicalMapping<Self, T> {
        let virtual_address = physical_address + self.physical_memory_offset as usize;
        unsafe {
            PhysicalMapping::new(
                physical_address,
                NonNull::new(virtual_address as *mut T).unwrap(),
                size,
                size,
                self.clone(),
            )
        }
    }

    fn unmap_physical_region<T>(_region: &PhysicalMapping<Self, T>) {}
}

pub fn power_off() {
    let offset = PHYSICAL_MEMORY_OFFSET.load(Ordering::SeqCst);
    if offset == 0 {
        crate::serial_println!("ACPI not initialized");
        return;
    }

    let handler = UmbraAcpiHandler::new(offset);
    let tables = unsafe { AcpiTables::search_for_rsdp_bios(handler) };

    if let Ok(tables) = tables {
        if let Ok(Some(fadt)) = unsafe { tables.get_sdt::<Fadt>(acpi::sdt::Signature::FADT) } {
            if let Ok(pm1a) = fadt.pm1a_control_block() {
                let mut port = Port::<u16>::new(pm1a.address as u16);
                unsafe {
                    port.write(0x2000);
                }
            } else {
                crate::serial_println!("No PM1A control block found");
            }
        } else {
            crate::serial_println!("No FADT found");
        }
    } else {
        crate::serial_println!("Failed to find ACPI tables");
    }
}
