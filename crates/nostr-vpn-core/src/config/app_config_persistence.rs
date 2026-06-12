impl AppConfig {
    pub fn generated() -> Self {
        Self::default()
    }

    pub fn generated_without_networks() -> Self {
        let mut config = Self::default();
        config.networks.clear();
        config.peer_aliases.clear();
        config
    }

    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let mut config: AppConfig =
            toml::from_str(&raw).with_context(|| "failed to parse config TOML")?;
        config.apply_load_migrations();
        hydrate_config_secrets(path, &mut config)?;
        config.ensure_defaults();
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.save_with_secret_persistence(path, SecretPersistence::Platform)
    }

    pub fn save_plaintext(&self, path: &Path) -> Result<()> {
        self.save(path)
    }

    pub fn delete_persisted_secrets_for_path(path: &Path) -> Result<()> {
        delete_config_secrets(path)
    }

    pub fn config_file_needs_secret_migration(path: &Path) -> Result<bool> {
        config_file_needs_secret_migration(path)
    }

    pub fn migrate_persisted_secrets(path: &Path) -> Result<bool> {
        if !Self::config_file_needs_secret_migration(path)? {
            return Ok(false);
        }

        let config = Self::load(path)?;
        config.save(path)?;
        Ok(true)
    }

    pub fn persisted_toml_for_path(&self, path: &Path) -> Result<String> {
        self.toml_with_secret_persistence(path, SecretPersistence::Platform)
    }

    pub fn plaintext_toml(&self) -> Result<String> {
        self.toml_with_secret_persistence(Path::new(""), SecretPersistence::Plaintext)
    }

    fn save_with_secret_persistence(
        &self,
        path: &Path,
        persistence: SecretPersistence,
    ) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let raw = self.toml_with_secret_persistence(path, persistence)?;
        write_config_file(path, raw.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    fn toml_with_secret_persistence(
        &self,
        path: &Path,
        persistence: SecretPersistence,
    ) -> Result<String> {
        let mut to_write = self.clone();
        to_write.ensure_defaults();
        to_write.canonicalize_user_facing_pubkeys();
        prepare_config_secrets_for_save(path, &mut to_write, persistence)?;

        toml::to_string_pretty(&to_write).with_context(|| "failed to encode TOML")
    }

}
