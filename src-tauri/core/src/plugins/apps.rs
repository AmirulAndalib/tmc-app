//! **App-scoped plugins** — how to install a mod for *this* game, and how to
//! launch it.
//!
//! Distinct from the plugin bundles in [`crate::plugins::registry`], and
//! deliberately so:
//!
//! | | Registry plugin | App plugin |
//! | --- | --- | --- |
//! | Where | `plugins/<id>/plugin.json` | `plugins/app/<slug>/*.json\|yaml` |
//! | Identified by | a reverse-DNS id the author picks | the GAME it handles |
//! | Approved | per bundle, by fingerprint | per game, once |
//! | Answers | "what can this plugin do?" | "where do this game's mods go, and how do its sandboxes work?" |
//!
//! A registry plugin is a thing a user chose to install. An app plugin is a
//! *rule for a game* — "a Minecraft mod is a `.jar` that goes in `mods/`" — and
//! the app ships one for every game it supports. There are dozens of them and
//! they are all tiny, so making each one a full bundle with its own id, its own
//! approval and its own directory would be ceremony with no payoff.
//!
//! **The safety model is unchanged.** Everything here compiles down to the same
//! [`Step`] vocabulary, executed by the same [`Executor`] through the same
//! [`Jail`]. An app plugin cannot express anything a registry plugin cannot:
//! no shell, no absolute path, no environment read. What it adds is only the
//! *selection* — which rule applies to which game, kind and file.
//!
//! [`Executor`]: crate::plugins::steps::Executor
//! [`Jail`]: crate::plugins::jail::Jail
//!
//! # Layout
//!
//! ```text
//! plugins/app/
//!   minecraft/
//!     manage_mod.json
//!     manage_asset.yaml
//!     launch.json
//!     sandbox.json                       ← presets, options, deploy strategies
//!     resourcepacks/manage_asset.json    ← recursive; subdirectories are fine
//!     disabled/manage_mod.json           ← IGNORED, entirely
//!   gtav/
//!     manage_mod.yaml
//! ```
//!
//! The directory name is the app's **URL slug** from the website, lower-cased
//! (`minecraft`, `gtav`) — which is unique per app and is what the API hands
//! down on every subscription. Using the slug rather than the numeric id means
//! a hand-authored file is readable, and means the same file works against a
//! development database whose ids differ.
//!
//! `disabled/` is skipped at any depth. That is the whole mechanism for turning
//! a rule off without deleting it, and it is a directory rather than a flag in
//! the file because the point is to be able to move a file without editing it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::plugins::manifest::{FsGrant, FsRoot, Manifest, Permissions, Step};

/// How deep the recursive scan goes. A rule five directories down is a mistake,
/// not a layout.
const MAX_DEPTH: u8 = 6;

/// Cap on one app plugin file. These are hand-authored rules, not data.
const MAX_FILE_BYTES: u64 = 512 * 1024;

/// Cap on how many files one game may declare, so a stray directory cannot
/// turn startup into a filesystem walk.
const MAX_FILES_PER_APP: usize = 64;

/// What a file declares.
///
/// Taken from the file's STEM, up to the first `.` — so `manage_mod.json` and
/// `manage_mod.forge.yaml` are both mod rules, which is how a game gets several
/// (see [`AppPluginFile::matches`] for how one is chosen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppPluginKind {
    ManageMod,
    ManageAsset,
    ManageCollection,
    Launch,
    /// `sandbox.json` — how this game's sandboxes behave: which deployment
    /// strategies work for it, which presets it offers, and which options its
    /// launch rule understands.
    Sandbox,
}

impl AppPluginKind {
    fn from_stem(stem: &str) -> Option<Self> {
        // Only the part before the first `.`, so a variant suffix is allowed.
        let head = stem.split('.').next().unwrap_or(stem);

        match head {
            "manage_mod" => Some(Self::ManageMod),
            "manage_asset" => Some(Self::ManageAsset),
            "manage_collection" => Some(Self::ManageCollection),
            "launch" => Some(Self::Launch),
            "sandbox" => Some(Self::Sandbox),
            _ => None,
        }
    }

    /// The content kind this rule installs, or `None` for a file that installs
    /// nothing.
    pub fn content_kind(self) -> Option<&'static str> {
        match self {
            Self::ManageMod => Some("mod"),
            Self::ManageAsset => Some("asset"),
            Self::ManageCollection => Some("collection"),
            Self::Launch | Self::Sandbox => None,
        }
    }

    /// Does this file describe installing something?
    pub fn is_manage(self) -> bool {
        matches!(
            self,
            Self::ManageMod | Self::ManageAsset | Self::ManageCollection
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManageMod => "manage_mod",
            Self::ManageAsset => "manage_asset",
            Self::ManageCollection => "manage_collection",
            Self::Launch => "launch",
            Self::Sandbox => "sandbox",
        }
    }
}

/// Which subscriptions a rule applies to.
///
/// All conditions must hold. Every field is optional and an empty `Match` — the
/// default — matches everything, which is the right behaviour for a game with
/// exactly one rule.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Match {
    /// Lower-case file extensions, without the dot: `["jar"]`, `["zip", "7z"]`.
    #[serde(default)]
    pub extensions: Vec<String>,

    /// Substrings that must appear (case-insensitively) in the file name.
    /// For the games where a loader is identifiable only from the filename.
    #[serde(default)]
    pub name_contains: Vec<String>,

    /// Install loaders this rule is for (`forge`, `fabric`, …), matched against
    /// the install's `loader` field. Empty means "any".
    #[serde(default)]
    pub loaders: Vec<String>,
}

impl Match {
    /// Does this rule apply?
    ///
    /// `file_name` is the release file's name (may be empty when unknown, in
    /// which case an extension condition cannot be satisfied and the rule is
    /// skipped — better than guessing).
    pub fn applies(&self, file_name: &str, loader: Option<&str>) -> bool {
        let lower = file_name.to_ascii_lowercase();

        if !self.extensions.is_empty() {
            let ext = lower.rsplit('.').next().unwrap_or_default();

            // A dotless name has no extension; `rsplit` would otherwise hand
            // back the whole name and match `["jar"]` against a file called
            // literally `jar`.
            if !lower.contains('.') || !self.extensions.iter().any(|e| e == ext) {
                return false;
            }
        }

        if !self.name_contains.is_empty()
            && !self
                .name_contains
                .iter()
                .any(|needle| lower.contains(&needle.to_ascii_lowercase()))
        {
            return false;
        }

        if !self.loaders.is_empty() {
            let Some(loader) = loader else {
                return false;
            };

            let loader = loader.to_ascii_lowercase();

            if !self
                .loaders
                .iter()
                .any(|l| l.to_ascii_lowercase() == loader)
            {
                return false;
            }
        }

        true
    }

    /// How specific this rule is. Used to pick between two that both match: the
    /// one with more conditions wins, because it was written for a narrower
    /// case.
    fn specificity(&self) -> usize {
        self.extensions.len() + self.name_contains.len() + self.loaders.len()
    }
}

/// The permissions an app plugin declares.
///
/// The same shape as a registry plugin's, and validated identically — because
/// it is compiled into one before anything runs. Spelled out per file rather
/// than granted implicitly by being an app plugin: "this rule writes to the
/// game's `mods/` folder and downloads from our CDN" is exactly what a reader
/// of the file needs to be able to see without cross-referencing anything.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppPluginPermissions {
    #[serde(default)]
    pub fs: Vec<FsGrant>,
    #[serde(default)]
    pub net: Vec<String>,
}

/// A launch specification: how to start this game.
///
/// **This is the one place in the whole plugin system that starts a process,**
/// and it is bounded to the point of being almost inexpressive:
///
///   * `exec` is a path RELATIVE to the game directory. There is no variant for
///     an absolute path, and the same [`crate::plugins::jail`] rules that
///     stop an installer escaping the jail are applied to it.
///   * `uri` is the alternative for games that launch through a client
///     (`steam://rungameid/271590`). It is handed to the OS opener, never to a
///     shell, and is checked against a scheme allow-list.
///   * Arguments are a VECTOR, filled from a fixed placeholder table. There is
///     no shell, so quoting, `;`, backticks and `$()` are inert bytes.
///
/// A launch spec still requires the user's confirmation the first time it
/// resolves to a given command line — see `launch_preview` on the app side.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchSpec {
    /// Executable, relative to the game directory. Mutually exclusive with
    /// `uri`; exactly one must be set.
    #[serde(default)]
    pub exec: Option<String>,

    /// Per-platform override for `exec` (`windows`, `macos`, `linux`). One game
    /// is one directory but three binaries.
    #[serde(default)]
    pub exec_platform: BTreeMap<String, String>,

    /// A URI handed to the OS (`steam://…`). Mutually exclusive with `exec`.
    #[serde(default)]
    pub uri: Option<String>,

    /// Working directory, relative to the game directory. Defaults to the game
    /// directory itself.
    #[serde(default)]
    pub cwd: Option<String>,

    /// Argument templates, in order. `{placeholder}` is filled from the launch
    /// context; an argument whose placeholders are ALL unresolved is dropped
    /// rather than passed through literally — see `LaunchPlan::build`.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables to set for the child. Values are templates too.
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// Arguments contributed by an install option, when that option is set.
    ///
    /// Keyed by the option name (`width`, `fullscreen`, `memoryMb`, …). This is
    /// what makes "resolution" mean `--width 1920 --height 1080` for one game
    /// and `-w 1920 -h 1080` for another without either being hardcoded.
    #[serde(default)]
    pub option_args: BTreeMap<String, Vec<String>>,
}

// ------------------------------------------------------------------ Sandbox

/// How this game's sandboxes behave.
///
/// The declarative half of the mod manager. Everything a game needs to say
/// about profiles that is not "where does a mod file go" lives here, in a file
/// anybody can write, so supporting a new game is a JSON file rather than a
/// release.
///
/// It is the PDF blueprint's "sandbox strategy matrix" plus two things that
/// blueprint left implicit: the OPTIONS a game understands (a launcher that
/// can set Minecraft's heap size but not a dedicated server's tick rate is
/// hardcoded to one game) and the PRESETS that make a new sandbox one click
/// instead of six fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SandboxSpec {
    #[serde(default)]
    pub deploy: DeploySpec,

    /// Ready-made sandboxes, offered when creating one.
    #[serde(default)]
    pub presets: Vec<PresetSpec>,

    /// The settings this game's launch rule understands, and how to edit them.
    #[serde(default)]
    pub options: Vec<OptionSpec>,

    /// How to recognise this game on a machine — see [`crate::detect`].
    #[serde(default)]
    pub detect: DetectSpec,
}

/// How to find this game's install folder without asking.
///
/// Every field is a hint, and detection only ever SUGGESTS: a folder found
/// through one of these still goes through [`crate::anchor::validate_root`]
/// before it can become a jail anchor, exactly as a hand-typed one does.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DetectSpec {
    /// Steam app ids, as strings — `["271590"]`. The most reliable hint there
    /// is: exact, language-independent, and unchanged by a rename.
    #[serde(default)]
    pub steam_app_ids: Vec<String>,

    /// Epic's `AppName` values.
    #[serde(default)]
    pub epic_app_names: Vec<String>,

    /// GOG product ids, as strings.
    #[serde(default)]
    pub gog_product_ids: Vec<String>,

    /// Display names to match. Compared with punctuation, case and edition
    /// suffixes removed, so `GRAND THEFT AUTO V™` matches
    /// `Grand Theft Auto V`.
    #[serde(default)]
    pub names: Vec<String>,

    /// A file that must be in the folder for it to be this game — `GTA5.exe`.
    ///
    /// What stops a name match being wrong. A folder matched only by name and
    /// with a marker declared must have it; an exact launcher-id match is not
    /// second-guessed.
    #[serde(default)]
    pub markers: Vec<String>,

    /// Folders to look in directly, keyed by platform (`windows`, `macos`,
    /// `linux`, `android`, `ios`, or `any` for all of them).
    ///
    /// Supports `~/`, `<drives>/`, `<programFiles>/`, `<appData>/` and
    /// `<localAppData>/` prefixes; anything else must be absolute. This is the
    /// escape hatch for what no launcher records — `~/.minecraft`, a dedicated
    /// server somebody unpacked by hand.
    #[serde(default)]
    pub paths: BTreeMap<String, Vec<String>>,
}

impl DetectSpec {
    pub fn is_empty(&self) -> bool {
        self.steam_app_ids.is_empty()
            && self.epic_app_names.is_empty()
            && self.gog_product_ids.is_empty()
            && self.names.is_empty()
            && self.paths.is_empty()
    }
}

/// Which deployment strategies make sense for this game.
///
/// Advisory, not enforcement: the engine's own capability probe decides what
/// is POSSIBLE on this machine, and this decides what is SENSIBLE for this
/// game. Both have to agree before a strategy is offered.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploySpec {
    /// `direct`, `hardlink`, `symlink` or `usvfs`. Absent means the app's own
    /// default (hard links).
    #[serde(default)]
    pub default_strategy: Option<String>,

    /// Empty means "any the machine supports".
    #[serde(default)]
    pub supported_strategies: Vec<String>,

    /// The game's anti-cheat, when it has one that matters.
    ///
    /// `kernel` is the load-bearing value: EAC, BattlEye and Vanguard all watch
    /// for API hooking and for a game folder whose files are not where they
    /// should be. A game declaring it gets Direct as its default and a warning
    /// on every other strategy, because the cost of guessing wrong here is
    /// somebody's account, not a failed install.
    #[serde(default)]
    pub anti_cheat: Option<String>,

    /// Where this game keeps mods, for the UI to show. Informational — the
    /// install rules are what actually place files.
    #[serde(default)]
    pub mod_targets: Vec<ModTarget>,

    /// Shown verbatim when this game's sandbox settings are opened.
    #[serde(default)]
    pub notes: Vec<String>,
}

impl DeploySpec {
    /// Does this game permit `strategy`?
    pub fn allows(&self, strategy: &str) -> bool {
        self.supported_strategies.is_empty()
            || self
                .supported_strategies
                .iter()
                .any(|s| s.eq_ignore_ascii_case(strategy))
    }

    /// Does this game run kernel-level anti-cheat?
    pub fn kernel_anti_cheat(&self) -> bool {
        self.anti_cheat
            .as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case("kernel"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModTarget {
    /// Free text: `engine_plugin`, `archive_mod`, `resource_pack`.
    pub r#type: String,
    /// Relative to the game directory.
    pub rel_path: String,
}

/// A ready-made sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresetSpec {
    /// Stable within one game. Recorded on the sandbox so the UI can say which
    /// preset it came from.
    pub id: String,
    pub label: String,

    #[serde(default)]
    pub description: Option<String>,

    /// `client`, `server` or `shared`.
    #[serde(default)]
    pub environment: Option<String>,

    #[serde(default)]
    pub game_version: Option<String>,
    /// `forge`, `fabric`, `neoforge`, `quilt`, …
    #[serde(default)]
    pub loader: Option<String>,

    #[serde(default)]
    pub strategy: Option<String>,

    /// Option values this preset sets. Merged over the option defaults.
    #[serde(default)]
    pub options: BTreeMap<String, serde_json::Value>,

    #[serde(default)]
    pub launch_args: Vec<String>,
}

/// How one setting is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OptionType {
    Int,
    Text,
    Bool,
    Select,
}

/// One setting a game's launch rule understands.
///
/// This is a *form description*, and deliberately nothing more. It cannot
/// express a condition, a computation or a dependency between fields — a
/// settings schema that can do those is a program, and the whole plugin model
/// is built on plugins not being programs. A game needing one of those needs a
/// second preset instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptionSpec {
    /// The placeholder name. `memoryMb` fills `{memoryMb}` and selects the
    /// `memoryMb` entry in the launch rule's `optionArgs`.
    pub key: String,
    pub label: String,

    #[serde(default)]
    pub description: Option<String>,

    pub r#type: OptionType,

    #[serde(default)]
    pub default: Option<serde_json::Value>,

    /// `int` only.
    #[serde(default)]
    pub min: Option<i64>,
    #[serde(default)]
    pub max: Option<i64>,
    #[serde(default)]
    pub step: Option<i64>,
    /// Suffix shown beside the field: `MB`, `FPS`.
    #[serde(default)]
    pub unit: Option<String>,

    /// `select` only.
    #[serde(default)]
    pub choices: Vec<OptionChoice>,

    /// Show only for these sandbox environments. Empty means all of them — a
    /// dedicated server has no resolution and a client has no tick rate, and
    /// showing both to both is how a settings screen becomes noise.
    #[serde(default)]
    pub environments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptionChoice {
    pub value: String,
    pub label: String,
}

/// Caps. These are hand-authored files describing a settings form, so the
/// numbers are "more than any real game needs" rather than tuned.
const MAX_PRESETS: usize = 128;
const MAX_OPTIONS: usize = 64;
const MAX_CHOICES: usize = 64;

/// A placeholder name: what `{key}` can be.
///
/// Restricted to the alphabet a launch template can actually reference, so a
/// key containing `{`, `}` or whitespace cannot produce a template that is
/// unfillable in a way nobody can see.
fn is_valid_option_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 48
        && key.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl SandboxSpec {
    fn validate(&self) -> AppResult<()> {
        if self.presets.len() > MAX_PRESETS || self.options.len() > MAX_OPTIONS {
            return Err(AppError::invalid(
                "A sandbox file declares too many presets or options.",
            ));
        }

        for strategy in self
            .deploy
            .supported_strategies
            .iter()
            .chain(self.deploy.default_strategy.iter())
        {
            if crate::deploy::Strategy::parse(strategy).is_none() {
                return Err(AppError::invalid(format!(
                    "'{strategy}' is not a deployment strategy this app knows."
                )));
            }
        }

        if let Some(default) = &self.deploy.default_strategy {
            if !self.deploy.allows(default) {
                return Err(AppError::invalid(format!(
                    "'{default}' is the default strategy but is not in supportedStrategies."
                )));
            }
        }

        let mut seen: Vec<&str> = Vec::new();

        for preset in &self.presets {
            if preset.id.trim().is_empty() || preset.id.len() > 64 {
                return Err(AppError::invalid("A preset needs a short, non-empty id."));
            }

            if seen.contains(&preset.id.as_str()) {
                return Err(AppError::invalid(format!(
                    "Two presets share the id '{}'.",
                    preset.id
                )));
            }

            seen.push(&preset.id);

            if let Some(env) = &preset.environment {
                if !matches!(env.as_str(), "client" | "server" | "shared") {
                    return Err(AppError::invalid(format!(
                        "'{env}' is not a sandbox environment (client, server or shared)."
                    )));
                }
            }

            if let Some(strategy) = &preset.strategy {
                if crate::deploy::Strategy::parse(strategy).is_none() {
                    return Err(AppError::invalid(format!(
                        "'{strategy}' is not a deployment strategy this app knows."
                    )));
                }
            }
        }

        for option in &self.options {
            if !is_valid_option_key(&option.key) {
                return Err(AppError::invalid(format!(
                    "'{}' is not a usable option key — letters, digits and '_' only.",
                    option.key
                )));
            }

            if option.choices.len() > MAX_CHOICES {
                return Err(AppError::invalid("An option declares too many choices."));
            }

            if option.r#type == OptionType::Select && option.choices.is_empty() {
                return Err(AppError::invalid(format!(
                    "Option '{}' is a select with no choices.",
                    option.key
                )));
            }

            if let (Some(min), Some(max)) = (option.min, option.max) {
                if min > max {
                    return Err(AppError::invalid(format!(
                        "Option '{}' has a minimum above its maximum.",
                        option.key
                    )));
                }
            }
        }

        Ok(())
    }

    /// The option values a fresh sandbox starts with.
    pub fn default_options(&self) -> BTreeMap<String, serde_json::Value> {
        self.options
            .iter()
            .filter_map(|o| o.default.clone().map(|d| (o.key.clone(), d)))
            .collect()
    }

    /// Clamp a set of option values to what the schema declares.
    ///
    /// Applied on every write. The values reach the cloud and come back, and a
    /// sandbox's options are the one part of it a second client could have
    /// written — so "the server said 900 GB of heap" is answered here rather
    /// than by a game that refuses to start.
    ///
    /// An option the schema does not declare is DROPPED. That is deliberate:
    /// the schema is what the game's launch rule reads, so a key not in it can
    /// only ever be noise, and keeping it would mean the sandbox's options grow
    /// forever as games change.
    pub fn clamp_options(
        &self,
        values: &BTreeMap<String, serde_json::Value>,
    ) -> BTreeMap<String, serde_json::Value> {
        let mut out = BTreeMap::new();

        for spec in &self.options {
            let Some(value) = values.get(&spec.key) else {
                continue;
            };

            let clamped = match spec.r#type {
                OptionType::Int => value.as_i64().map(|n| {
                    let n = spec.min.map_or(n, |min| n.max(min));
                    let n = spec.max.map_or(n, |max| n.min(max));

                    serde_json::Value::from(n)
                }),
                OptionType::Bool => value.as_bool().map(serde_json::Value::from),
                OptionType::Text => value
                    .as_str()
                    .map(|s| serde_json::Value::from(s.chars().take(512).collect::<String>())),
                OptionType::Select => value.as_str().and_then(|s| {
                    spec.choices
                        .iter()
                        .find(|c| c.value == s)
                        .map(|c| serde_json::Value::from(c.value.clone()))
                }),
            };

            if let Some(clamped) = clamped {
                out.insert(spec.key.clone(), clamped);
            }
        }

        out
    }

    pub fn preset(&self, id: &str) -> Option<&PresetSpec> {
        self.presets.iter().find(|p| p.id == id)
    }
}

/// A manage rule: how to install and uninstall one content item for this game.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManageSpec {
    #[serde(default)]
    pub install: Vec<Step>,
    #[serde(default)]
    pub uninstall: Vec<Step>,
}

/// One parsed file under `plugins/app/<slug>/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppPluginFile {
    /// Must equal 1. Same contract as the registry manifest: a file from a
    /// newer app version is refused rather than half-understood.
    pub manifest_version: u32,

    /// Human-readable, for the log and the UI. Not an identity.
    #[serde(default)]
    pub label: Option<String>,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub r#match: Match,

    #[serde(default)]
    pub permissions: AppPluginPermissions,

    #[serde(default)]
    pub manage: Option<ManageSpec>,

    #[serde(default)]
    pub launch: Option<LaunchSpec>,

    #[serde(default)]
    pub sandbox: Option<SandboxSpec>,

    // ---------------------------------------------------- Filled by the loader
    /// The kind, from the file name. `skip_deserializing` because it is not a
    /// key in the file — writing one would be a second, contradictable source
    /// of truth for something the path already says.
    #[serde(skip_deserializing, default = "default_kind")]
    pub kind: AppPluginKind,

    /// Path relative to `plugins/app/`, e.g. `minecraft/manage_mod.json`. The
    /// stable name this rule is identified by in the log and in the UI.
    #[serde(skip_deserializing, default)]
    pub source: String,

    /// The app slug this file was found under.
    #[serde(skip_deserializing, default)]
    pub slug: String,
}

fn default_kind() -> AppPluginKind {
    AppPluginKind::ManageMod
}

impl AppPluginFile {
    fn validate(&self) -> AppResult<()> {
        if self.manifest_version != 1 {
            return Err(AppError::invalid(format!(
                "App plugin version {} is not supported by this app version.",
                self.manifest_version
            )));
        }

        match self.kind {
            AppPluginKind::Launch => {
                let Some(launch) = &self.launch else {
                    return Err(AppError::invalid("A launch file must declare `launch`."));
                };

                let has_exec = launch.exec.is_some() || !launch.exec_platform.is_empty();

                if has_exec == launch.uri.is_some() {
                    return Err(AppError::invalid(
                        "A launch file must declare exactly one of `exec` and `uri`.",
                    ));
                }

                if let Some(uri) = &launch.uri {
                    if !is_allowed_launch_uri(uri) {
                        return Err(AppError::invalid(format!(
                            "'{uri}' is not a launch URI this app will open."
                        )));
                    }
                }

                if launch.args.len() > 64 {
                    return Err(AppError::invalid("Launch spec has too many arguments."));
                }
            }
            AppPluginKind::Sandbox => {
                let Some(sandbox) = &self.sandbox else {
                    return Err(AppError::invalid("A sandbox file must declare `sandbox`."));
                };

                sandbox.validate()?;
            }
            _ => {
                let Some(manage) = &self.manage else {
                    return Err(AppError::invalid("A manage file must declare `manage`."));
                };

                if manage.install.len() > 128 || manage.uninstall.len() > 128 {
                    return Err(AppError::invalid("App plugin has too many steps."));
                }

                /*
                 * A plan with no write grant can only fail, loudly, halfway
                 * through. Same check the registry manifest makes, for the same
                 * reason: catching it at LOAD time means the log says "this rule
                 * is broken" rather than "installing Foo failed".
                 */
                if !manage.install.is_empty() && !self.permissions.fs.iter().any(|g| g.write) {
                    return Err(AppError::invalid(
                        "App plugin has install steps but no writable filesystem permission.",
                    ));
                }
            }
        }

        Ok(())
    }

    /// Does this rule apply to a given file and install?
    pub fn matches(&self, file_name: &str, loader: Option<&str>) -> bool {
        self.r#match.applies(file_name, loader)
    }

    /// Compile into a [`Manifest`], so the existing jail and executor can
    /// run it unchanged.
    ///
    /// The synthetic id is derived from the source path, which is what makes
    /// `pluginData` scoped per rule — two rules for one game get separate
    /// scratch directories, and neither can read the other's staging area.
    pub fn as_manifest(&self) -> Manifest {
        Manifest {
            manifest_version: crate::plugins::manifest::MANIFEST_VERSION,
            id: self.synthetic_id(),
            name: self
                .label
                .clone()
                .unwrap_or_else(|| format!("{} ({})", self.kind.as_str(), self.slug)),
            version: "1".into(),
            author: "The Modding Community".into(),
            description: self.description.clone(),
            homepage: None,
            // App plugins are selected by SLUG, not by numeric app id — the
            // slug is what the directory name is. Leaving this empty means
            // `jail_for` still receives the app id from the caller, which is
            // where the game directory lookup happens.
            apps: vec![],
            permissions: Permissions {
                fs: self.launch_grants(),
                net: self.permissions.net.clone(),
                query: false,
            },
            installer: None,
            server_query: None,
            theme: None,
        }
    }

    /// The rule's own grants, plus the one a launch rule always needs.
    ///
    /// A launch spec's only paths are an executable and a working directory
    /// inside the game folder, and `launch::plan` resolves both with
    /// `write: false`. Making an author declare "I may READ the folder I am
    /// launching out of" is a footgun with no security value: the grant is
    /// read-only, the jail still refuses anything outside the root, and the
    /// only thing the requirement actually achieves is that a launch rule with
    /// an empty `permissions` block — which is what every example and every
    /// obvious first draft has — fails to resolve its own executable.
    ///
    /// The write side is untouched. A launch rule cannot be given one here,
    /// and one it declares for itself still has to be declared.
    fn launch_grants(&self) -> Vec<FsGrant> {
        let mut grants = self.permissions.fs.clone();

        if self.kind == AppPluginKind::Launch
            && !grants
                .iter()
                .any(|g| g.root == FsRoot::GameDir && g.path.is_empty() && !g.write)
        {
            grants.push(FsGrant {
                root: FsRoot::GameDir,
                path: String::new(),
                write: false,
            });
        }

        grants
    }

    /// A stable, path-safe id for this rule.
    ///
    /// Every character outside the plugin-id alphabet becomes `-`, so
    /// `minecraft/resourcepacks/manage_asset.yaml` becomes
    /// `app.minecraft-resourcepacks-manage-asset-yaml`. It has to satisfy
    /// `is_valid_plugin_id`, because it names a directory under `plugin-data`.
    fn synthetic_id(&self) -> String {
        let mut out = String::from("app.");

        for ch in self.source.chars() {
            if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
                out.push(ch);
            } else if ch.is_ascii_uppercase() {
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push('-');
            }
        }

        // A trailing or doubled separator is legal in the alphabet but ugly;
        // more importantly a leading/trailing `.` is REFUSED by the validator,
        // and `-` is not, which is why the replacement char is `-`.
        out.truncate(96);

        out
    }
}

/// URI schemes an app plugin may ask the OS to open in order to launch a game.
///
/// A closed list, because "open this URI" is a request to hand a string to
/// whatever program claimed a scheme — `file:` would open a local executable,
/// `http:` a browser, and a bare word could be anything. These four are game
/// clients and nothing else.
const LAUNCH_SCHEMES: [&str; 4] = [
    "steam://",
    "com.epicgames.launcher://",
    "uplay://",
    "origin://",
];

fn is_allowed_launch_uri(uri: &str) -> bool {
    let lower = uri.to_ascii_lowercase();

    if lower.len() > 512 {
        return false;
    }

    // Control characters and whitespace in a URI handed to a platform opener
    // are how one string becomes two arguments on at least one platform.
    if lower
        .chars()
        .any(|c| c.is_control() || c == '"' || c == '\'')
    {
        return false;
    }

    LAUNCH_SCHEMES.iter().any(|s| lower.starts_with(s))
}

/// Every app plugin on this machine, keyed by slug.
#[derive(Debug, Default)]
pub struct AppPlugins {
    by_slug: BTreeMap<String, Vec<AppPluginFile>>,
    /// Files that failed to parse, so Settings → Plugins can show WHY rather
    /// than silently offering fewer games than the folder contains.
    errors: Vec<(String, String)>,
}

impl AppPlugins {
    /// Scan `<root>/app/` and parse everything under it.
    ///
    /// Never fails as a whole: one broken file is recorded in `errors` and the
    /// rest load. A single malformed YAML must not cost the user every other
    /// game's install support.
    pub fn load(root: &Path) -> Self {
        let mut out = Self::default();

        let base = root.join("app");

        let Ok(entries) = std::fs::read_dir(&base) else {
            return out;
        };

        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };

            if !meta.is_dir() {
                continue;
            }

            let Some(slug) = entry.file_name().to_str().map(|s| s.to_ascii_lowercase()) else {
                continue;
            };

            if slug == "disabled" || !is_valid_slug(&slug) {
                continue;
            }

            let mut files = Vec::new();

            out.walk(&entry.path(), &slug, &slug, 0, &mut files);

            files.sort_by(|a, b| {
                // Most specific first, so `choose` can take the first match.
                b.r#match
                    .specificity()
                    .cmp(&a.r#match.specificity())
                    .then_with(|| a.source.cmp(&b.source))
            });

            if !files.is_empty() {
                out.by_slug.insert(slug, files);
            }
        }

        out
    }

    fn walk(
        &mut self,
        dir: &Path,
        slug: &str,
        rel: &str,
        depth: u8,
        files: &mut Vec<AppPluginFile>,
    ) {
        if depth > MAX_DEPTH || files.len() >= MAX_FILES_PER_APP {
            return;
        }

        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };

            // Never follow a symlink into the tree. A `disabled -> ../enabled`
            // link would otherwise re-enable everything it points at, and a
            // link out of the plugins directory would load arbitrary files.
            if meta.file_type().is_symlink() {
                continue;
            }

            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };

            if meta.is_dir() {
                // The whole disable mechanism, at any depth.
                if name.eq_ignore_ascii_case("disabled") {
                    continue;
                }

                self.walk(
                    &entry.path(),
                    slug,
                    &format!("{rel}/{name}"),
                    depth + 1,
                    files,
                );

                continue;
            }

            if meta.len() > MAX_FILE_BYTES {
                self.errors.push((
                    format!("{rel}/{name}"),
                    "File is too large to be a plugin rule.".into(),
                ));
                continue;
            }

            let source = format!("{rel}/{name}");

            match parse_file(&entry.path(), &source, slug) {
                Ok(Some(parsed)) => files.push(parsed),
                Ok(None) => {}
                Err(e) => self.errors.push((source, e.to_string())),
            }

            if files.len() >= MAX_FILES_PER_APP {
                return;
            }
        }
    }

    /// The rules for one game.
    pub fn for_slug(&self, slug: &str) -> &[AppPluginFile] {
        self.by_slug
            .get(&slug.to_ascii_lowercase())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// The best rule of a kind for a given file and install, or `None`.
    ///
    /// The list is pre-sorted most-specific-first, so this is the first match —
    /// which means a rule written for `.jar` beats the catch-all, and two rules
    /// of equal specificity are broken by path so the answer is stable.
    pub fn choose(
        &self,
        slug: &str,
        kind: AppPluginKind,
        file_name: &str,
        loader: Option<&str>,
    ) -> Option<&AppPluginFile> {
        self.for_slug(slug)
            .iter()
            .find(|f| f.kind == kind && f.matches(file_name, loader))
    }

    /// The launch spec for a game, if it has one.
    pub fn launch_for(&self, slug: &str, loader: Option<&str>) -> Option<&AppPluginFile> {
        self.for_slug(slug)
            .iter()
            .find(|f| f.kind == AppPluginKind::Launch && f.matches("", loader))
    }

    /// Which games have at least one manage rule — what the UI calls "supported".
    pub fn managed_slugs(&self) -> Vec<String> {
        self.by_slug
            .iter()
            .filter(|(_, files)| files.iter().any(|f| f.kind.is_manage()))
            .map(|(slug, _)| slug.clone())
            .collect()
    }

    /// Every game with at least one rule of any kind.
    pub fn slugs(&self) -> Vec<String> {
        self.by_slug.keys().cloned().collect()
    }

    /// How this game's sandboxes behave, if it says.
    ///
    /// A game with no `sandbox.json` is not unsupported — it gets the app's own
    /// defaults, which is the right answer for the many games where "put the
    /// file in `mods/` and hard-link it" is the whole story.
    pub fn sandbox_spec(&self, slug: &str) -> Option<&SandboxSpec> {
        self.for_slug(slug)
            .iter()
            .find(|f| f.kind == AppPluginKind::Sandbox)
            .and_then(|f| f.sandbox.as_ref())
    }

    pub fn errors(&self) -> &[(String, String)] {
        &self.errors
    }

    pub fn is_empty(&self) -> bool {
        self.by_slug.is_empty()
    }
}

/// A game directory name we will look inside.
///
/// The same alphabet as an app URL on the website, which is what these mirror.
/// Strict because it becomes a path component and is read off the filesystem.
fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Parse one file. `Ok(None)` means "not a plugin rule" (a README, an icon).
fn parse_file(path: &Path, source: &str, slug: &str) -> AppResult<Option<AppPluginFile>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    let is_json = ext == "json";
    let is_yaml = ext == "yaml" || ext == "yml";

    if !is_json && !is_yaml {
        return Ok(None);
    }

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let Some(kind) = AppPluginKind::from_stem(&stem) else {
        return Ok(None);
    };

    let raw = std::fs::read_to_string(path)
        .map_err(|e| AppError::invalid(format!("Could not read the rule: {e}")))?;

    /*
     * Both formats deserialise into the SAME struct, which is the whole reason
     * both can be supported for the price of one code path. YAML is a superset
     * of JSON in principle, but not in serde's implementation of it — a JSON
     * file with a tab in its indentation parses as JSON and fails as YAML — so
     * the extension decides rather than a try-both fallback that would report
     * the wrong error.
     */
    let mut parsed: AppPluginFile = if is_json {
        serde_json::from_str(&raw).map_err(|e| AppError::invalid(format!("Invalid JSON: {e}")))?
    } else {
        serde_yaml_ng::from_str(&raw)
            .map_err(|e| AppError::invalid(format!("Invalid YAML: {e}")))?
    };

    parsed.kind = kind;
    parsed.source = source.to_string();
    parsed.slug = slug.to_string();

    parsed.validate()?;

    // The compiled manifest has to satisfy the registry validator too — that is
    // what proves an app plugin cannot express anything a registry plugin
    // cannot. Checked at LOAD time rather than at run time so a broken rule is
    // reported before a user asks it to install something.
    let manifest = parsed.as_manifest();

    if !crate::plugins::manifest::is_valid_plugin_id(&manifest.id) {
        return Err(AppError::invalid(
            "Rule path does not produce a usable plugin id.",
        ));
    }

    Ok(Some(parsed))
}

/// Where app plugins live, given the app's plugins directory.
pub fn app_plugin_dir(plugins_root: &Path) -> PathBuf {
    plugins_root.join("app")
}

/// The default grants a rule gets when it declares none.
///
/// Deliberately NOT applied automatically — a rule with no grants and install
/// steps is refused at load. This exists for the shipped examples and for the
/// docs, so "what should I write here?" has one answer.
pub fn default_grants() -> Vec<FsGrant> {
    vec![
        FsGrant {
            root: FsRoot::PluginData,
            path: String::new(),
            write: true,
        },
        FsGrant {
            root: FsRoot::GameDir,
            path: String::new(),
            write: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    const MOD_JSON: &str = r#"{
        "manifestVersion": 1,
        "label": "Minecraft jar mod",
        "match": { "extensions": ["jar"] },
        "permissions": {
            "fs": [
                { "root": "pluginData", "path": "", "write": true },
                { "root": "gameDir", "path": "mods", "write": true }
            ],
            "net": ["example.com"]
        },
        "manage": {
            "install": [
                { "action": "download", "url": "{fileUrl}", "to": { "root": "pluginData", "path": "staging/{fileName}" } },
                { "action": "copy", "from": { "root": "pluginData", "path": "staging/{fileName}" }, "to": { "root": "gameDir", "path": "mods/{fileName}" } }
            ],
            "uninstall": [
                { "action": "remove", "path": { "root": "gameDir", "path": "mods/{fileName}" } }
            ]
        }
    }"#;

    const MOD_YAML: &str = r#"
manifestVersion: 1
label: GTA V asi mod
match:
  extensions: [asi, dll]
permissions:
  fs:
    - root: pluginData
      path: ""
      write: true
    - root: gameDir
      path: ""
      write: true
manage:
  install:
    - action: download
      url: "{fileUrl}"
      to: { root: pluginData, path: "staging/{fileName}" }
    - action: copy
      from: { root: pluginData, path: "staging/{fileName}" }
      to: { root: gameDir, path: "{fileName}" }
  uninstall:
    - action: remove
      path: { root: gameDir, path: "{fileName}" }
"#;

    #[test]
    fn json_and_yaml_both_load_into_the_same_shape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "app/minecraft/manage_mod.json", MOD_JSON);
        write(root, "app/gtav/manage_mod.yaml", MOD_YAML);

        let plugins = AppPlugins::load(root);

        assert!(plugins.errors().is_empty(), "{:?}", plugins.errors());
        assert_eq!(plugins.for_slug("minecraft").len(), 1);
        assert_eq!(plugins.for_slug("gtav").len(), 1);

        let mc = plugins
            .choose("minecraft", AppPluginKind::ManageMod, "cool.jar", None)
            .expect("matched");

        assert_eq!(mc.kind, AppPluginKind::ManageMod);
        assert_eq!(mc.manage.as_ref().expect("manage").install.len(), 2);

        let gta = plugins
            .choose("gtav", AppPluginKind::ManageMod, "trainer.asi", None)
            .expect("matched");

        assert_eq!(gta.manage.as_ref().expect("manage").uninstall.len(), 1);
    }

    /// A launch rule can resolve its own executable with an empty permission
    /// block.
    ///
    /// The regression: it could not. `launch::plan` resolves the executable
    /// through the jail as `gameDir`, and a rule declaring no grants had no
    /// `gameDir` — so every launch rule that did not think to ask for read
    /// access to the folder it was launching out of failed with a jail error,
    /// including the shipped example. The grant added for it is READ-ONLY,
    /// which is the whole reason requiring it bought nothing.
    #[test]
    fn a_launch_rule_may_read_the_folder_it_launches_from() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(
            root,
            "app/minecraft/launch.json",
            r#"{
                "manifestVersion": 1,
                "permissions": {},
                "launch": { "exec": "run.sh" }
            }"#,
        );

        let plugins = AppPlugins::load(root);
        let rule = plugins.launch_for("minecraft", None).expect("loaded");
        let manifest = rule.as_manifest();

        let grant = manifest
            .permissions
            .fs
            .iter()
            .find(|g| g.root == FsRoot::GameDir)
            .expect("a launch rule gets a gameDir grant");

        assert!(grant.path.is_empty());
        assert!(!grant.write, "read-only, and that is the point");
    }

    /// The grant is for LAUNCH rules only. A manage rule that wants to write
    /// into the game folder still has to say so.
    #[test]
    fn a_manage_rule_gets_no_free_grant() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(
            root,
            "app/minecraft/manage_mod.json",
            r#"{
                "manifestVersion": 1,
                "match": { "extensions": ["jar"] },
                "permissions": {},
                "manage": { "install": [], "uninstall": [] }
            }"#,
        );

        let plugins = AppPlugins::load(root);

        let rule = plugins
            .choose("minecraft", AppPluginKind::ManageMod, "x.jar", None)
            .expect("loaded");

        assert!(
            rule.as_manifest().permissions.fs.is_empty(),
            "nothing is granted to a rule that asked for nothing"
        );
    }

    /// A launch rule that DOES declare a write grant keeps it. The implicit
    /// grant is an addition, not a replacement — a rule that legitimately
    /// writes a config before starting the game must not silently lose the
    /// permission it declared.
    #[test]
    fn an_explicit_grant_on_a_launch_rule_survives() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(
            root,
            "app/minecraft/launch.json",
            r#"{
                "manifestVersion": 1,
                "permissions": {
                    "fs": [{ "root": "gameDir", "path": "config", "write": true }]
                },
                "launch": { "exec": "run.sh" }
            }"#,
        );

        let plugins = AppPlugins::load(root);
        let manifest = plugins
            .launch_for("minecraft", None)
            .expect("loaded")
            .as_manifest();

        assert_eq!(manifest.permissions.fs.len(), 2);
        assert!(manifest
            .permissions
            .fs
            .iter()
            .any(|g| g.path == "config" && g.write));
    }

    #[test]
    fn a_disabled_directory_is_ignored_at_any_depth() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "app/minecraft/disabled/manage_mod.json", MOD_JSON);
        write(
            root,
            "app/minecraft/packs/disabled/manage_asset.json",
            MOD_JSON,
        );

        let plugins = AppPlugins::load(root);

        assert!(plugins.for_slug("minecraft").is_empty());
    }

    #[test]
    fn subdirectories_are_scanned() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "app/minecraft/packs/manage_asset.json", MOD_JSON);

        let plugins = AppPlugins::load(root);

        let found = plugins.for_slug("minecraft");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, AppPluginKind::ManageAsset);
        assert_eq!(found[0].source, "minecraft/packs/manage_asset.json");
    }

    #[test]
    fn the_most_specific_rule_wins() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // A catch-all and a `.jar` rule. The `.jar` one must win for a jar and
        // the catch-all must still serve a zip.
        let catch_all = MOD_JSON.replace(r#""match": { "extensions": ["jar"] },"#, "");

        write(root, "app/minecraft/manage_mod.json", MOD_JSON);
        write(root, "app/minecraft/manage_mod.any.json", &catch_all);

        let plugins = AppPlugins::load(root);

        assert_eq!(plugins.for_slug("minecraft").len(), 2);

        let jar = plugins
            .choose("minecraft", AppPluginKind::ManageMod, "a.jar", None)
            .expect("matched");

        assert_eq!(jar.source, "minecraft/manage_mod.json");

        let zip = plugins
            .choose("minecraft", AppPluginKind::ManageMod, "a.zip", None)
            .expect("matched");

        assert_eq!(zip.source, "minecraft/manage_mod.any.json");
    }

    #[test]
    fn install_steps_without_a_write_grant_are_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        let no_write = MOD_JSON.replace(r#""write": true"#, r#""write": false"#);

        write(root, "app/minecraft/manage_mod.json", &no_write);

        let plugins = AppPlugins::load(root);

        assert!(plugins.for_slug("minecraft").is_empty());
        assert_eq!(plugins.errors().len(), 1);
    }

    #[test]
    fn a_broken_file_does_not_take_the_others_down() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "app/minecraft/manage_mod.json", MOD_JSON);
        write(root, "app/minecraft/manage_asset.json", "{ not json");

        let plugins = AppPlugins::load(root);

        assert_eq!(plugins.for_slug("minecraft").len(), 1);
        assert_eq!(plugins.errors().len(), 1);
    }

    #[test]
    fn launch_uris_are_restricted_to_game_clients() {
        assert!(is_allowed_launch_uri("steam://rungameid/271590"));
        assert!(is_allowed_launch_uri("uplay://launch/720/0"));

        for bad in [
            "file:///bin/sh",
            "http://example.com",
            "javascript:alert(1)",
            "steam://run\nid",
            "steam://run\"id",
            "notsteam://x",
        ] {
            assert!(!is_allowed_launch_uri(bad), "{bad} should be refused");
        }
    }

    #[test]
    fn a_launch_file_must_name_exactly_one_target() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(
            root,
            "app/minecraft/launch.json",
            r#"{ "manifestVersion": 1, "launch": { "exec": "run.sh", "uri": "steam://x" } }"#,
        );

        let plugins = AppPlugins::load(root);

        assert!(plugins.for_slug("minecraft").is_empty());
        assert_eq!(plugins.errors().len(), 1);
    }

    #[test]
    fn extension_matching_needs_a_real_extension() {
        let m = Match {
            extensions: vec!["jar".into()],
            ..Default::default()
        };

        assert!(m.applies("mod.jar", None));
        assert!(m.applies("MOD.JAR", None));
        // A file literally named `jar`, with no dot, must not match.
        assert!(!m.applies("jar", None));
        assert!(!m.applies("", None));
        assert!(!m.applies("mod.zip", None));
    }

    #[test]
    fn loader_conditions_need_a_loader() {
        let m = Match {
            loaders: vec!["fabric".into()],
            ..Default::default()
        };

        assert!(m.applies("a.jar", Some("Fabric")));
        assert!(!m.applies("a.jar", Some("forge")));
        assert!(!m.applies("a.jar", None));
    }

    /// The shipped examples are the documentation for this format. A field
    /// renamed without updating them would ship a reference that does not load.
    ///
    /// The teeth are in the SECOND half: parsing only proves the JSON/YAML is
    /// well-formed, while building each rule's real jail and resolving every
    /// step path through it is what catches a rule whose grants and step paths
    /// disagree — the exact mistake an author copying an example inherits.
    #[test]
    fn the_shipped_app_examples_all_load_and_resolve() {
        // `core/` → `src-tauri/` → repo root.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");

        let plugins = AppPlugins::load(&root);

        assert!(
            plugins.errors().is_empty(),
            "example app plugins failed to load: {:?}",
            plugins.errors()
        );

        let slugs = plugins.managed_slugs();

        assert!(
            slugs.contains(&"minecraft".to_string()) && slugs.contains(&"gtav".to_string()),
            "expected minecraft and gtav examples, found {slugs:?}"
        );

        /*
         * The sandbox examples are the reference for that format, and the one
         * mistake they most invite is a strategy a preset names that the game's
         * own `supportedStrategies` excludes — which loads fine and then
         * refuses at deploy time, on somebody's machine rather than here.
         */
        for slug in ["minecraft", "gtav"] {
            let spec = plugins
                .sandbox_spec(slug)
                .unwrap_or_else(|| panic!("{slug} should ship a sandbox.json"));

            assert!(
                !spec.presets.is_empty() && !spec.options.is_empty(),
                "{slug}'s sandbox example should demonstrate both halves"
            );

            for preset in &spec.presets {
                if let Some(strategy) = &preset.strategy {
                    assert!(
                        spec.deploy.allows(strategy),
                        "{slug} preset '{}' wants {strategy}, which the game does not support",
                        preset.id
                    );
                }
            }

            // Every option a preset sets must survive its own schema, or the
            // preset silently produces a sandbox missing half its settings.
            for preset in &spec.presets {
                let clamped = spec.clamp_options(&preset.options);

                assert_eq!(
                    clamped.len(),
                    preset.options.len(),
                    "{slug} preset '{}' sets an option its schema drops",
                    preset.id
                );
            }
        }

        // The `disabled/` example must not have been loaded.
        assert!(
            !plugins
                .for_slug("minecraft")
                .iter()
                .any(|f| f.source.contains("/disabled/")),
            "a rule under disabled/ was loaded"
        );

        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&game).expect("game dir");

        let roots = crate::plugins::JailRoots {
            data: tmp.path().join("data"),
            cache: tmp.path().join("cache"),
        };

        let mut settings = crate::settings::AppSettings::default();
        settings
            .game_dirs
            .insert("1".into(), game.display().to_string());

        let mut checked = 0;

        for slug in &slugs {
            for rule in plugins.for_slug(slug) {
                let Some(manage) = &rule.manage else { continue };

                let manifest = rule.as_manifest();

                let jail = crate::plugins::jail_for(&manifest, &roots, &settings, Some(1))
                    .unwrap_or_else(|e| panic!("{} jail: {e}", rule.source));

                for step in manage.install.iter().chain(&manage.uninstall) {
                    for (path_ref, write) in step_paths(step) {
                        jail.resolve(path_ref, write).unwrap_or_else(|e| {
                            panic!("{} step path '{}': {e}", rule.source, path_ref.path)
                        });
                    }
                }

                checked += 1;
            }
        }

        assert!(checked >= 2, "expected several manage examples");
    }

    /// Every `PathRef` a step touches, and whether it is written to.
    fn step_paths(step: &Step) -> Vec<(&crate::plugins::manifest::PathRef, bool)> {
        match step {
            Step::Download { to, .. } => vec![(to, true)],
            Step::Extract { from, to, .. } => vec![(from, false), (to, true)],
            Step::Copy { from, to } => vec![(from, false), (to, true)],
            Step::Move { from, to } => vec![(from, true), (to, true)],
            Step::Mkdir { path }
            | Step::Remove { path }
            | Step::WriteText { path, .. }
            | Step::PatchJson { path, .. } => vec![(path, true)],
        }
    }

    // ------------------------------------------------------------- Sandboxes

    const SANDBOX_JSON: &str = r#"{
        "manifestVersion": 1,
        "label": "Minecraft sandboxes",
        "sandbox": {
            "deploy": {
                "defaultStrategy": "symlink",
                "supportedStrategies": ["direct", "symlink", "hardlink"],
                "modTargets": [{ "type": "loader_mod", "relPath": "mods" }]
            },
            "presets": [
                {
                    "id": "fabric-1.21",
                    "label": "Fabric 1.21",
                    "environment": "client",
                    "gameVersion": "1.21",
                    "loader": "fabric",
                    "options": { "memoryMb": 4096 }
                },
                {
                    "id": "server-neoforge-1.21",
                    "label": "NeoForge server 1.21",
                    "environment": "server",
                    "gameVersion": "1.21",
                    "loader": "neoforge"
                }
            ],
            "options": [
                {
                    "key": "memoryMb",
                    "label": "Memory",
                    "type": "int",
                    "min": 512,
                    "max": 32768,
                    "default": 2048,
                    "unit": "MB"
                },
                { "key": "jvmArgs", "label": "JVM arguments", "type": "text" },
                {
                    "key": "renderer",
                    "label": "Renderer",
                    "type": "select",
                    "choices": [
                        { "value": "gl", "label": "OpenGL" },
                        { "value": "vk", "label": "Vulkan" }
                    ]
                }
            ]
        }
    }"#;

    fn spec_from(body: &str) -> Option<SandboxSpec> {
        let tmp = tempfile::tempdir().expect("tempdir");

        write(tmp.path(), "app/minecraft/sandbox.json", body);

        AppPlugins::load(tmp.path())
            .sandbox_spec("minecraft")
            .cloned()
    }

    #[test]
    fn a_sandbox_file_declares_presets_options_and_strategies() {
        let spec = spec_from(SANDBOX_JSON).expect("loaded");

        assert_eq!(spec.deploy.default_strategy.as_deref(), Some("symlink"));
        assert!(spec.deploy.allows("direct"));
        assert!(!spec.deploy.allows("usvfs"));
        assert!(!spec.deploy.kernel_anti_cheat());

        assert_eq!(spec.presets.len(), 2);
        assert_eq!(
            spec.preset("fabric-1.21").map(|p| p.loader.as_deref()),
            Some(Some("fabric"))
        );

        // Defaults come from the option schema, not from a hardcoded table.
        assert_eq!(
            spec.default_options().get("memoryMb"),
            Some(&serde_json::json!(2048))
        );
    }

    /// A sandbox file is loaded alongside the game's manage rules rather than
    /// replacing them, and it must not make the game look unsupported.
    #[test]
    fn a_sandbox_file_does_not_count_as_install_support() {
        let tmp = tempfile::tempdir().expect("tempdir");

        write(tmp.path(), "app/minecraft/sandbox.json", SANDBOX_JSON);

        let plugins = AppPlugins::load(tmp.path());

        assert!(plugins.sandbox_spec("minecraft").is_some());
        assert!(
            plugins.managed_slugs().is_empty(),
            "a game with no manage rule cannot install anything"
        );

        write(tmp.path(), "app/minecraft/manage_mod.json", MOD_JSON);

        let plugins = AppPlugins::load(tmp.path());

        assert_eq!(plugins.managed_slugs(), vec!["minecraft".to_string()]);
        assert!(plugins.sandbox_spec("minecraft").is_some());
    }

    /// The clamp is the reason a hostile or stale cloud payload cannot hand a
    /// game an absurd command line.
    #[test]
    fn option_values_are_clamped_to_the_schema() {
        let spec = spec_from(SANDBOX_JSON).expect("loaded");

        let clamped = spec.clamp_options(&BTreeMap::from([
            ("memoryMb".into(), serde_json::json!(999_999_999i64)),
            ("renderer".into(), serde_json::json!("not-a-choice")),
            ("jvmArgs".into(), serde_json::json!("-XX:+UseG1GC")),
            ("unknownKey".into(), serde_json::json!("whatever")),
        ]));

        assert_eq!(clamped.get("memoryMb"), Some(&serde_json::json!(32768)));
        assert_eq!(
            clamped.get("jvmArgs"),
            Some(&serde_json::json!("-XX:+UseG1GC"))
        );
        // A value outside the declared choices is dropped, not passed through.
        assert!(!clamped.contains_key("renderer"));
        // And so is a key the game never declared.
        assert!(!clamped.contains_key("unknownKey"));

        let low = spec.clamp_options(&BTreeMap::from([("memoryMb".into(), serde_json::json!(1))]));

        assert_eq!(low.get("memoryMb"), Some(&serde_json::json!(512)));
    }

    #[test]
    fn a_sandbox_file_naming_an_unknown_strategy_is_refused() {
        for bad in [
            r#"{"deploy":{"supportedStrategies":["telepathy"]}}"#,
            r#"{"deploy":{"defaultStrategy":"symlink","supportedStrategies":["direct"]}}"#,
            r#"{"presets":[{"id":"a","label":"A","environment":"somewhere"}]}"#,
            r#"{"presets":[{"id":"a","label":"A"},{"id":"a","label":"B"}]}"#,
            r#"{"options":[{"key":"1bad","label":"L","type":"int"}]}"#,
            r#"{"options":[{"key":"pick","label":"L","type":"select"}]}"#,
            r#"{"options":[{"key":"n","label":"L","type":"int","min":10,"max":1}]}"#,
        ] {
            let body = format!(r#"{{ "manifestVersion": 1, "sandbox": {bad} }}"#);

            assert!(spec_from(&body).is_none(), "{bad} should not have loaded");
        }
    }

    /// A game with kernel anti-cheat has to be able to say so. Getting this
    /// wrong costs somebody an account, not an install.
    #[test]
    fn kernel_anti_cheat_is_declarable() {
        let spec = spec_from(
            r#"{
                "manifestVersion": 1,
                "sandbox": {
                    "deploy": {
                        "antiCheat": "kernel",
                        "defaultStrategy": "direct",
                        "supportedStrategies": ["direct"]
                    }
                }
            }"#,
        )
        .expect("loaded");

        assert!(spec.deploy.kernel_anti_cheat());
        assert!(!spec.deploy.allows("hardlink"));
    }

    #[test]
    fn synthetic_ids_are_valid_plugin_ids() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "app/minecraft/packs/manage_asset.yaml", MOD_YAML);

        let plugins = AppPlugins::load(root);
        let file = &plugins.for_slug("minecraft")[0];

        let id = file.as_manifest().id;

        assert!(
            crate::plugins::manifest::is_valid_plugin_id(&id),
            "{id} is not a valid plugin id"
        );
    }
}
