// SPDX-License-Identifier: MIT
use super::super::*;
use std::ffi::OsString;

impl Registration {
    pub(in crate::file_manager) fn enabled_v1(&self, executable: &Path) -> io::Result<bool> {
        if !self
            .owner()?
            .is_some_and(|owner| same_path(&owner, executable))
        {
            return Ok(false);
        }
        let expected = self.command()?;
        let verb = Some(Value::text(OsStr::new(VERB))?);
        for class in CLASSES {
            if registry::get(&self.shell(class), "")? != verb
                || registry::get(&format!(r"{}\command", self.verb(class)), "")? != expected
            {
                return Ok(false);
            }
        }
        Ok(expected.is_some()
            && registry::get(&self.run, RUN_VALUE)? == registry::get(&self.state, "WatchCommand")?)
    }

    pub(in crate::file_manager) fn disable_v1(&self, executable: &Path) -> io::Result<()> {
        if !self
            .owner()?
            .is_some_and(|owner| same_path(&owner, executable))
        {
            return Ok(());
        }
        let command = self
            .command()?
            .ok_or_else(|| io::Error::other("Missing file-manager command backup"))?;
        let verb = Some(Value::text(OsStr::new(VERB))?);
        for class in CLASSES {
            let own_command = registry::get(&format!(r"{}\command", self.verb(class)), "")?
                .as_ref()
                == Some(&command);
            if own_command {
                if registry::get(&self.shell(class), "")? == verb {
                    let backup = registry::get(&self.state, class)?
                        .ok_or_else(|| io::Error::other("Missing file-manager default backup"))?;
                    registry::set(&self.shell(class), "", backup.original()?.as_ref())?;
                }
                registry::remove_tree(&self.verb(class))?;
            }
        }
        if let Some(expected) = registry::get(&self.state, "WatchCommand")?
            && registry::get(&self.run, RUN_VALUE)?.as_ref() == Some(&expected)
        {
            let backup = registry::get(&self.state, "RunBackup")?
                .ok_or_else(|| io::Error::other("Missing startup command backup"))?;
            registry::set(&self.run, RUN_VALUE, backup.original()?.as_ref())?;
        }
        registry::remove_tree(&self.state)?;
        Ok(())
    }

    pub(in crate::file_manager) fn enable_v1(
        &self,
        executable: &Path,
        proxy: &Path,
        label: &str,
    ) -> io::Result<()> {
        if !executable.is_absolute()
            || !proxy.is_absolute()
            || !executable.is_file()
            || !proxy.is_file()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "File-manager executables must exist at absolute paths",
            ));
        }
        let mut command = OsString::from("\"");
        command.push(proxy.as_os_str());
        command.push("\" --open \"%1\\.\"");
        let command = Value::text(&command)?;
        let mut watch = OsString::from("\"");
        watch.push(proxy.as_os_str());
        watch.push("\" --watch");
        let watch = Value::text(&watch)?;
        if let Some(owner) = self.owner()? {
            if same_path(&owner, executable) && self.command()?.as_ref() == Some(&command) {
                // Resume an interrupted registration without replacing its backup.
            } else {
                self.disable_v1(&owner)?;
            }
        }
        if self.owner()?.is_none() {
            for class in CLASSES {
                if registry::get(&format!(r"{}\command", self.verb(class)), "")?.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "Another registration owns the Strata folder verb",
                    ));
                }
            }
            // Flush the complete rollback journal before overriding a shell default.
            for class in CLASSES {
                let original = registry::get(&self.shell(class), "")?;
                registry::set(&self.state, class, Some(&Value::backup(original)))?;
            }
            registry::set(&self.state, "Command", Some(&command))?;
            registry::set(&self.state, "Proxy", Some(&Value::text(proxy.as_os_str())?))?;
            registry::set(
                &self.state,
                "RunBackup",
                Some(&Value::backup(registry::get(&self.run, RUN_VALUE)?)),
            )?;
            registry::set(&self.state, "WatchCommand", Some(&watch))?;
            registry::set(
                &self.state,
                "Executable",
                Some(&Value::text(executable.as_os_str())?),
            )?;
            registry::flush(&self.state)?;
        }
        let result = (|| {
            registry::set(&self.run, RUN_VALUE, Some(&watch))?;
            for class in CLASSES {
                registry::set(
                    &format!(r"{}\command", self.verb(class)),
                    "",
                    Some(&command),
                )?;
                registry::set(
                    &self.verb(class),
                    "",
                    Some(&Value::text(OsStr::new(label))?),
                )?;
                registry::set(
                    &self.shell(class),
                    "",
                    Some(&Value::text(OsStr::new(VERB))?),
                )?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = self.disable_v1(executable);
        }
        result
    }
}
