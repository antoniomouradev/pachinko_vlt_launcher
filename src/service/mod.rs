mod linux;

use anyhow::Result;

pub fn install_service() -> Result<()> {
    linux::install_service()
}

pub fn uninstall_service() -> Result<()> {
    linux::uninstall_service()
}
