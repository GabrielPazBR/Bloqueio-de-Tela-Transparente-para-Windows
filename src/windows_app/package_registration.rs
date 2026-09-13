use anyhow::{Result, bail};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::*;

const KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\BloqueioTransparente";
type RegistryValue = (u32, Vec<u8>);

/// Restores ARP metadata if a later installation step fails.
pub struct Registration {
    key: HKEY,
    previous: Vec<(Vec<u16>, Option<RegistryValue>)>,
    created: bool,
    committed: bool,
}

impl Registration {
    pub fn open() -> Result<Self> {
        let mut key = null_mut();
        let mut disposition = 0;
        check(unsafe {
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                wide(KEY).as_ptr(),
                0,
                null(),
                0,
                KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
                null(),
                &mut key,
                &mut disposition,
            )
        })?;
        Ok(Self {
            key,
            previous: Vec::new(),
            created: disposition == REG_CREATED_NEW_KEY,
            committed: false,
        })
    }

    fn set(&mut self, name: &str, kind: u32, bytes: &[u8]) -> Result<()> {
        let name = wide(name);
        let mut old_type = 0;
        let mut size = 0;
        let status = unsafe {
            RegQueryValueExW(
                self.key,
                name.as_ptr(),
                null(),
                &mut old_type,
                null_mut(),
                &mut size,
            )
        };
        let previous = if status == ERROR_FILE_NOT_FOUND {
            None
        } else {
            check(status)?;
            let mut bytes = vec![0; size as usize];
            check(unsafe {
                RegQueryValueExW(
                    self.key,
                    name.as_ptr(),
                    null(),
                    &mut old_type,
                    bytes.as_mut_ptr(),
                    &mut size,
                )
            })?;
            bytes.truncate(size as usize);
            Some((old_type, bytes))
        };
        self.previous.push((name.clone(), previous));
        check(unsafe {
            RegSetValueExW(
                self.key,
                name.as_ptr(),
                0,
                kind,
                bytes.as_ptr(),
                bytes.len() as u32,
            )
        })
    }

    pub fn text(&mut self, name: &str, value: &str) -> Result<()> {
        let bytes: Vec<u8> = wide(value).into_iter().flat_map(u16::to_le_bytes).collect();
        self.set(name, REG_SZ, &bytes)
    }

    pub fn number(&mut self, name: &str, value: u32) -> Result<()> {
        self.set(name, REG_DWORD, &value.to_le_bytes())
    }

    pub fn commit(mut self) {
        self.committed = true;
    }

    pub fn rollback(&mut self) -> Result<()> {
        let mut failures = Vec::new();
        for (name, previous) in self.previous.iter().rev() {
            let status = unsafe {
                if let Some((kind, bytes)) = previous {
                    RegSetValueExW(
                        self.key,
                        name.as_ptr(),
                        0,
                        *kind,
                        bytes.as_ptr(),
                        bytes.len() as u32,
                    )
                } else {
                    RegDeleteValueW(self.key, name.as_ptr())
                }
            };
            if status != ERROR_FILE_NOT_FOUND
                && let Err(error) = check(status)
            {
                failures.push(error.to_string());
            }
        }
        self.committed = true;
        if self.created {
            unsafe { RegCloseKey(self.key) };
            self.key = null_mut();
            if let Err(error) = remove() {
                failures.push(error.to_string());
            }
        }
        if !failures.is_empty() {
            bail!("falha ao restaurar o registro: {}", failures.join("; "));
        }
        Ok(())
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.rollback();
        }
        if !self.key.is_null() {
            unsafe { RegCloseKey(self.key) };
        }
    }
}

pub fn remove() -> Result<()> {
    let status =
        unsafe { RegDeleteKeyExW(HKEY_LOCAL_MACHINE, wide(KEY).as_ptr(), KEY_WOW64_64KEY, 0) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(status)
}

fn check(status: u32) -> Result<()> {
    if status != ERROR_SUCCESS {
        bail!(
            "registro de instalação: {}",
            std::io::Error::from_raw_os_error(status as i32)
        );
    }
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
