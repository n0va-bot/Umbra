use x86_64::instructions::port::Port;

pub struct Cmos {
    addr_port: Port<u8>,
    data_port: Port<u8>,
}

impl Cmos {
    pub fn new() -> Self {
        Self {
            addr_port: Port::new(0x70),
            data_port: Port::new(0x71),
        }
    }

    unsafe fn read_register(&mut self, reg: u8) -> u8 {
        unsafe {
            self.addr_port.write(reg);
            self.data_port.read()
        }
    }

    fn bcd_to_binary(bcd: u8) -> u8 {
        (bcd & 0x0F) + ((bcd / 16) * 10)
    }

    pub fn read_time(&mut self) -> (u8, u8, u8, u8, u8, u8) {
        unsafe {
            while self.read_register(0x0A) & 0x80 != 0 {
                core::hint::spin_loop();
            }

            let seconds = Self::bcd_to_binary(self.read_register(0x00));
            let minutes = Self::bcd_to_binary(self.read_register(0x02));

            let hours = Self::bcd_to_binary(self.read_register(0x04));

            let day = Self::bcd_to_binary(self.read_register(0x07));
            let month = Self::bcd_to_binary(self.read_register(0x08));
            let year = Self::bcd_to_binary(self.read_register(0x09));

            (year, month, day, hours, minutes, seconds)
        }
    }
}
