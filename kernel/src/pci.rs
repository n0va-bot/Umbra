use x86_64::instructions::port::Port;

fn pci_config_address(bus: u8, device: u8, func: u8, offset: u8) -> u32 {
    1 << 31
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((func as u32) << 8)
        | (offset as u32 & 0xFC)
}

unsafe fn pci_read_u32(bus: u8, device: u8, func: u8, offset: u8) -> u32 {
    let address = pci_config_address(bus, device, func, offset);
    let mut addr_port = Port::<u32>::new(0xCF8);
    let mut data_port = Port::<u32>::new(0xCFC);

    unsafe {
        addr_port.write(address);
        data_port.read()
    }
}

pub fn scan_buses() {
    for bus in 0..=255 {
        for device in 0..=31 {
            for func in 0..=7 {
                unsafe {
                    let reg0 = pci_read_u32(bus, device, func, 0);
                    let vendor_id = (reg0 & 0xFFFF) as u16;

                    if vendor_id != 0xFFFF {
                        let device_id = (reg0 >> 16) as u16;

                        let reg8 = pci_read_u32(bus, device, func, 8);
                        let class_code = (reg8 >> 24) as u8;
                        let subclass = (reg8 >> 16) as u8;

                        crate::serial_println!(
                            "Bus {:02X} | Dev {:02X} | Func {:02X} => Vendor: {:04X}, Device: {:04X} | Class: {:02X}, Sub: {:02X}",
                            bus,
                            device,
                            func,
                            vendor_id,
                            device_id,
                            class_code,
                            subclass
                        );
                    }
                }
            }
        }
    }
}
