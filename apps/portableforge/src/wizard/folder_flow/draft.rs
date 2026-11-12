use std::path::Path;

use portableforge_core::paf::{pad_package_version, valid_app_id, CATEGORIES};
use portableforge_core::{Distribution, PafMetadata};

use super::Page;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Language {
    English,
    Multilingual,
}

impl Language {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Multilingual => "Multilingual",
        }
    }
}

pub(super) struct Draft {
    pub(super) name: String,
    pub(super) publisher: String,
    pub(super) homepage: String,
    pub(super) description: String,
    pub(super) version: String,
    pub(super) id: String,
    pub(super) category: &'static str,
    pub(super) language: Option<Language>,
    pub(super) open_source: bool,
    pub(super) freeware: bool,
    pub(super) commercial_use: bool,
    pub(super) requires_admin: bool,
    pub(super) rights: bool,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            name: String::new(),
            publisher: String::new(),
            homepage: String::new(),
            description: String::new(),
            version: String::new(),
            id: String::new(),
            category: "Utilities",
            language: None,
            open_source: false,
            freeware: false,
            commercial_use: false,
            requires_admin: false,
            rights: false,
        }
    }
}

fn has_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty()
        && !value.chars().any(char::is_control)
        && value.chars().count() <= limit
}

fn package_version(display: &str) -> Option<String> {
    pad_package_version(display)
        .filter(|version| version.split('.').all(|part| part.parse::<u16>().is_ok()))
}

pub(super) fn suggested_id(name: &str) -> String {
    let base: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(32)
        .collect();
    if base.to_ascii_lowercase().ends_with("portable") {
        base
    } else if base.is_empty() {
        "AppPortable".into()
    } else {
        format!("{base}Portable")
    }
}

impl Draft {
    pub(super) fn blocker(&self) -> Option<(Page, &'static str)> {
        if !has_text(&self.name, 120) {
            Some((Page::App, "Enter a name for the app."))
        } else if !has_text(&self.publisher, 120) {
            Some((Page::App, "Enter the name of the app’s publisher."))
        } else if !has_text(&self.version, 40) || package_version(&self.version).is_none() {
            Some((
                Page::App,
                "Enter the app version in numbers, such as 2.4 or 2.4.1.",
            ))
        } else if !valid_homepage(self.homepage.trim()) {
            Some((Page::About, "Enter the address of the app’s website."))
        } else if !has_text(&self.description, 512) {
            Some((Page::About, "Write a short description of the app."))
        } else if !valid_app_id(&self.id)
            || !self
                .id
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            || self.id.ends_with('.')
            || self.id.len() > 48
        {
            Some((
                Page::Package,
                "Choose a package name with no spaces, such as WidgetPortable.",
            ))
        } else if !CATEGORIES.contains(&self.category) {
            Some((Page::Package, "Choose an app category."))
        } else if self.language.is_none() {
            Some((
                Page::Package,
                "Choose English or Multilingual for the installer.",
            ))
        } else if !self.rights {
            Some((
                Page::Review,
                "Confirm you have permission to include these app files.",
            ))
        } else {
            None
        }
    }

    pub(super) fn metadata(&self, selected: &Path) -> Option<PafMetadata> {
        let name = self.name.trim();
        let name = if name.to_ascii_lowercase().ends_with(" portable") {
            name.to_owned()
        } else {
            format!("{name} Portable")
        };
        Some(PafMetadata {
            name,
            app_id: self.id.clone(),
            base_app_name: "Program".into(),
            publisher: self.publisher.trim().into(),
            homepage: self.homepage.trim().into(),
            category: self.category.into(),
            description: self.description.trim().into(),
            language: self.language?.label().into(),
            shareable: self.rights,
            open_source: self.open_source,
            freeware: self.freeware,
            commercial_use: self.commercial_use,
            package_version: package_version(&self.version)?,
            display_version: self.version.trim().into(),
            requires_admin: self.requires_admin,
            main_exe: selected.to_str()?.replace('/', "\\"),
            main_exe_64bit: None,
            distribution: Distribution::Bundled {
                redistribution_asserted: self.rights,
            },
        })
    }
}

fn valid_homepage(value: &str) -> bool {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        portableforge_core::wizard_package::valid_homepage(value)
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        value.starts_with("https://") || value.starts_with("http://")
    }
}
