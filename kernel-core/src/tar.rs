pub struct TarArchive<'a> {
    data: &'a [u8],
}

pub struct TarEntry<'a> {
    pub name: &'a str,
    pub size: usize,
    pub data: &'a [u8],
}

impl<'a> TarArchive<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub fn iter(&self) -> TarIterator<'a> {
        TarIterator {
            data: self.data,
            offset: 0,
        }
    }
}

pub struct TarIterator<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Iterator for TarIterator<'a> {
    type Item = TarEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.offset + 512 <= self.data.len() {
            let header = &self.data[self.offset..self.offset + 512];

            if header.iter().all(|&b| b == 0) {
                return None;
            }

            let size_bytes = &header[124..136];
            let mut size = 0;
            for &b in size_bytes {
                if b >= b'0' && b <= b'7' {
                    size = size * 8 + (b - b'0') as usize;
                } else if b == 0 || b == b' ' {
                    if size > 0 || b == 0 {
                        break;
                    }
                }
            }

            let name_bytes = &header[0..100];
            let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(100);
            let name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("<invalid utf8>");

            let typeflag = header[156];

            let data_offset = self.offset + 512;
            let data_len = size;
            let next_offset = data_offset + (size + 511) & !511;

            self.offset = next_offset;

            if typeflag == b'0' || typeflag == 0 {
                if data_offset + data_len <= self.data.len() {
                    let entry_data = &self.data[data_offset..data_offset + data_len];
                    return Some(TarEntry {
                        name,
                        size,
                        data: entry_data,
                    });
                } else {
                    return None;
                }
            }
        }
        None
    }
}
