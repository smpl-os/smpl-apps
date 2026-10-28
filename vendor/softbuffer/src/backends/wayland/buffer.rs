use memmap2::MmapMut;
use std::{
    ffi::CStr,
    fs::File,
    io,
    os::unix::prelude::{AsFd, AsRawFd},
    slice,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use wayland_client::{
    protocol::{wl_buffer, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle,
};

use super::State;
use crate::error::SwResultExt;
use crate::SoftBufferError;

fn pixel_format() -> wl_shm::Format {
    if cfg!(feature = "wayland-alpha") {
        wl_shm::Format::Argb8888
    } else {
        wl_shm::Format::Xrgb8888
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn create_memfile() -> io::Result<File> {
    use rustix::fs::{MemfdFlags, SealFlags};

    let name = unsafe { CStr::from_bytes_with_nul_unchecked("softbuffer\0".as_bytes()) };
    let fd = rustix::fs::memfd_create(name, MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)?;
    rustix::fs::fcntl_add_seals(&fd, SealFlags::SHRINK | SealFlags::SEAL)?;
    Ok(File::from(fd))
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn create_memfile() -> io::Result<File> {
    use rustix::{fs::Mode, io::Errno, shm::OFlags};
    use std::iter;

    // Use a cached RNG to avoid hammering the thread local.
    let mut rng = fastrand::Rng::new();

    for _ in 0..=4 {
        let mut name = String::from("softbuffer-");
        name.extend(iter::repeat_with(|| rng.alphanumeric()).take(7));
        name.push('\0');

        let name = unsafe { CStr::from_bytes_with_nul_unchecked(name.as_bytes()) };
        // `CLOEXEC` is implied with `shm_open`
        let fd = rustix::shm::open(
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL,
            Mode::RWXU,
        );
        if !matches!(fd, Err(Errno::EXIST)) {
            let fd = fd?;
            rustix::shm::unlink(name)?;
            return Ok(File::from(fd));
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "No unused shared-memory name",
    ))
}

// Round size to use for pool for given dimensions, rounding up to power of 2
fn get_pool_size(width: i32, height: i32) -> io::Result<i32> {
    let size = (|| {
        let width = u32::try_from(width).ok().filter(|&v| v > 0)?;
        let height = u32::try_from(height).ok().filter(|&v| v > 0)?;
        let bytes = width.checked_mul(height)?.checked_mul(4)?;
        i32::try_from(bytes.checked_next_power_of_two()?).ok()
    })();
    size.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Wayland buffer dimensions exceed shared-memory limits",
        )
    })
}

unsafe fn map_file(file: &File) -> io::Result<MmapMut> {
    unsafe { MmapMut::map_mut(file.as_raw_fd()) }
}

#[derive(Debug)]
pub(super) struct WaylandBuffer {
    qh: QueueHandle<State>,
    tempfile: File,
    map: MmapMut,
    pool: wl_shm_pool::WlShmPool,
    pool_size: i32,
    buffer: wl_buffer::WlBuffer,
    pub width: i32,
    pub height: i32,
    released: Arc<AtomicBool>,
    pub age: u8,
}

impl WaylandBuffer {
    pub fn new(
        shm: &wl_shm::WlShm,
        width: i32,
        height: i32,
        qh: &QueueHandle<State>,
    ) -> Result<Self, SoftBufferError> {
        // Calculate size to use for shm pool
        let pool_size = get_pool_size(width, height).swbuf_err("Invalid Wayland buffer size")?;

        // Create an `mmap` shared memory
        let tempfile = create_memfile().swbuf_err("Failed to create Wayland shared memory")?;
        tempfile
            .set_len(pool_size as u64)
            .swbuf_err("Failed to size Wayland shared memory")?;
        let map =
            unsafe { map_file(&tempfile) }.swbuf_err("Failed to map Wayland shared memory")?;

        // Create wayland shm pool and buffer
        let pool = shm.create_pool(tempfile.as_fd(), pool_size, qh, ());
        let released = Arc::new(AtomicBool::new(true));
        let buffer = pool.create_buffer(
            0,
            width,
            height,
            width * 4,
            pixel_format(),
            qh,
            released.clone(),
        );

        Ok(Self {
            qh: qh.clone(),
            map,
            tempfile,
            pool,
            pool_size,
            buffer,
            width,
            height,
            released,
            age: 0,
        })
    }

    pub fn resize(&mut self, width: i32, height: i32) -> Result<(), SoftBufferError> {
        // If size is the same, there's nothing to do
        if self.width != width || self.height != height {
            // Grow pool, if needed
            let size = get_pool_size(width, height).swbuf_err("Invalid Wayland buffer size")?;
            if size > self.pool_size {
                self.tempfile
                    .set_len(size as u64)
                    .swbuf_err("Failed to grow Wayland shared memory")?;
                let map = unsafe { map_file(&self.tempfile) }
                    .swbuf_err("Failed to map resized Wayland shared memory")?;
                self.pool.resize(size);
                self.pool_size = size;
                self.map = map;
            }

            self.buffer.destroy();
            // Create buffer with correct size
            self.buffer = self.pool.create_buffer(
                0,
                width,
                height,
                width * 4,
                pixel_format(),
                &self.qh,
                self.released.clone(),
            );
            self.width = width;
            self.height = height;
            self.age = 0;
        }
        Ok(())
    }

    pub fn attach(&self, surface: &wl_surface::WlSurface) {
        self.released.store(false, Ordering::SeqCst);
        surface.attach(Some(&self.buffer), 0, 0);
    }

    pub fn released(&self) -> bool {
        self.released.load(Ordering::SeqCst)
    }

    fn len(&self) -> usize {
        self.width as usize * self.height as usize
    }

    pub unsafe fn mapped_mut(&mut self) -> &mut [u32] {
        unsafe { slice::from_raw_parts_mut(self.map.as_mut_ptr() as *mut u32, self.len()) }
    }
}

impl Drop for WaylandBuffer {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_feature_selects_the_actual_protocol_format() {
        assert_eq!(
            pixel_format(),
            if cfg!(feature = "wayland-alpha") {
                wl_shm::Format::Argb8888
            } else {
                wl_shm::Format::Xrgb8888
            }
        );
    }

    #[test]
    fn pool_size_is_bounded_before_protocol_or_memory_allocation() {
        assert_eq!(get_pool_size(200, 120).unwrap(), 131_072);
        assert_eq!(get_pool_size(3840, 2160).unwrap(), 33_554_432);
        for (w, h) in [(0, 120), (-1, 120), (i32::MAX, 1), (32_768, 32_768)] {
            assert!(get_pool_size(w, h).is_err());
        }
    }

    #[test]
    fn mapping_failure_is_returned_not_panicked() {
        let file = File::open(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
        assert!(
            unsafe { map_file(&file) }.is_err(),
            "read-only files cannot back a writable render buffer"
        );
    }

    #[test]
    fn shared_memory_is_sized_and_writable() {
        let file = create_memfile().unwrap();
        file.set_len(4096).unwrap();
        let mut map = unsafe { map_file(&file) }.unwrap();
        map[4095] = 255;
        assert_eq!(map.len(), 4096);
        assert_eq!(map[4095], 255);
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for State {
    fn event(
        _: &mut State,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, Arc<AtomicBool>> for State {
    fn event(
        _: &mut State,
        _: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        released: &Arc<AtomicBool>,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let wl_buffer::Event::Release = event {
            released.store(true, Ordering::SeqCst);
        }
    }
}
