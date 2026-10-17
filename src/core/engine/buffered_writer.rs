use anyhow::{Context, Result};
use std::sync::Arc;

#[cfg(windows)]
pub fn platform_write_all_at(
    file: &std::fs::File,
    mut offset: u64,
    mut bytes: &[u8],
) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    use std::os::windows::fs::FileExt;

    while !bytes.is_empty() {
        let written = file.seek_write(bytes, offset)?;
        if written == 0 {
            return Err(Error::new(
                ErrorKind::WriteZero,
                "failed to write whole buffer",
            ));
        }
        offset += written as u64;
        bytes = &bytes[written..];
    }
    Ok(())
}

#[cfg(unix)]
pub fn platform_write_all_at(
    file: &std::fs::File,
    mut offset: u64,
    mut bytes: &[u8],
) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    use std::os::unix::fs::FileExt;

    while !bytes.is_empty() {
        let written = file.write_at(bytes, offset)?;
        if written == 0 {
            return Err(Error::new(
                ErrorKind::WriteZero,
                "failed to write whole buffer",
            ));
        }
        offset += written as u64;
        bytes = &bytes[written..];
    }
    Ok(())
}

#[cfg(not(any(windows, unix)))]
pub fn platform_write_all_at(file: &std::fs::File, offset: u64, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::{Seek, Write};

    let mut file = file.try_clone()?;
    file.seek(std::io::SeekFrom::Start(offset))?;
    file.write_all(bytes)
}

pub async fn write_all_at(file: Arc<std::fs::File>, offset: u64, bytes: Vec<u8>) -> Result<()> {
    tokio::task::spawn_blocking(move || platform_write_all_at(&file, offset, &bytes))
        .await
        .context("offset write task panicked")??;
    Ok(())
}

pub struct AsyncBufferedWriter {
    file: Arc<std::fs::File>,
    buffer: Vec<u8>,
    capacity: usize,
    base_offset: u64,
}

impl AsyncBufferedWriter {
    pub fn new(file: Arc<std::fs::File>, capacity: usize, start_offset: u64) -> Self {
        Self {
            file,
            buffer: Vec::with_capacity(capacity),
            capacity,
            base_offset: start_offset,
        }
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        if self.buffer.len() + data.len() >= self.capacity {
            let available = self.capacity.saturating_sub(self.buffer.len());
            if available > 0 {
                self.buffer.extend_from_slice(&data[..available]);
                self.flush().await?;
                Box::pin(self.write(&data[available..])).await?;
            } else {
                self.flush().await?;
                Box::pin(self.write(data)).await?;
            }
        } else {
            self.buffer.extend_from_slice(data);
        }
        Ok(())
    }

    pub async fn flush(&mut self) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let data = std::mem::take(&mut self.buffer);
        let offset = self.base_offset;
        self.base_offset += data.len() as u64;
        self.buffer.reserve(self.capacity);

        write_all_at(self.file.clone(), offset, data).await?;
        Ok(())
    }
}
