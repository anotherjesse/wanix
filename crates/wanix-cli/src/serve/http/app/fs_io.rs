use wanix_fs::{FileSystem, FsError, FsResult, NormalizedPath, OpenOptions};

pub(super) fn create_dir_if_missing(fs: &dyn FileSystem, path: &str) -> FsResult<()> {
    match fs.create_dir(&normalized(path)?) {
        Ok(()) | Err(FsError::AlreadyExists) => Ok(()),
        Err(error) => Err(error),
    }
}

pub(super) fn truncate_file(fs: &dyn FileSystem, path: &str) -> FsResult<()> {
    let mut file = fs.open(
        &normalized(path)?,
        OpenOptions {
            write: true,
            create: true,
            truncate: true,
            ..OpenOptions::default()
        },
    )?;
    file.set_len(0).or_else(|error| match error {
        FsError::NotSupported => Ok(()),
        error => Err(error),
    })
}

pub(super) fn write_service_text(fs: &dyn FileSystem, path: &str, text: &str) -> FsResult<()> {
    write_all(
        fs,
        path,
        text.as_bytes(),
        OpenOptions {
            write: true,
            ..OpenOptions::default()
        },
    )
}

pub(super) fn read_text(fs: &dyn FileSystem, path: &str) -> FsResult<String> {
    Ok(String::from_utf8_lossy(&read_all(fs, path)?).into_owned())
}

pub(super) fn read_all(fs: &dyn FileSystem, path: &str) -> FsResult<Vec<u8>> {
    let mut file = fs.open(&normalized(path)?, OpenOptions::read())?;
    let mut body = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let len = file.read(&mut buffer)?;
        if len == 0 {
            return Ok(body);
        }
        body.extend_from_slice(&buffer[..len]);
    }
}

pub(super) fn normalized(path: &str) -> FsResult<NormalizedPath> {
    NormalizedPath::new(path)
}

fn write_all(fs: &dyn FileSystem, path: &str, bytes: &[u8], options: OpenOptions) -> FsResult<()> {
    let mut file = fs.open(&normalized(path)?, options)?;
    let mut written = 0;
    while written < bytes.len() {
        let len = file.write(&bytes[written..])?;
        if len == 0 {
            return Err(FsError::Other(format!("short write to {path}")));
        }
        written += len;
    }
    Ok(())
}
