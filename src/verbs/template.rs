//! The `template upgrade` verb (spec 006): the one local governed verb.
//!
//! Every other verb (tenants/stamp/fleet) is a thin wrapper over the
//! control-plane API. This one never touches the plane. It runs in a stamped
//! app checkout and orchestrates a chassis upgrade end to end: read
//! `template.toml`, bump the chassis package pins in `package.json`, refresh the
//! lockfile, run any template-owned codemods, run the contract's verify verb,
//! and commit on a branch. The CLI orchestrates; all structure knowledge stays
//! in the template and its packages (spec 006 summary). This is the boundary
//! that keeps the CLI from ever becoming a build daemon.
//!
//! Design notes (recorded in spec 006 §4 for the coherence guard):
//! - The template version and the chassis package version move in lockstep, so
//!   the resolved target is applied as the exact pin for each chassis package.
//! - The compat gate is the contract's own `[requires]` range: a target outside
//!   it is the "major upgrade requires the migration path" refusal, so the
//!   version policy lives in the template, not the CLI.
//! - Chassis packages are discovered from `[requires]` by unscoped name (so
//!   `node` is naturally skipped and `toolchain` resolves to `@enrahitu/…`),
//!   then every exact-pinned dependency in the discovered scope is bumped
//!   (catching companions like `@enrahitu/hiqlite-native`) with no hardcoded
//!   scope in the CLI.
//! - Every git/npm/node side effect sits behind [`Runner`] so `cargo test` runs
//!   fully offline (the spec 003 §1 discipline: never gate tests on the outside
//!   world). The pure planning is exercised directly.

use std::collections::BTreeMap;
use std::path::Path;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AppError, AppResult, EXIT_OPERATIONAL};
use crate::output::OutputFormat;

/// `template upgrade`, run against the stamped app in `dir` (the current working
/// directory). Resolves through the real [`ProcessRunner`]; the offline core is
/// [`run`], which the tests drive with a fake runner.
pub fn upgrade(
    dir: &Path,
    to: Option<&str>,
    dry_run: bool,
    no_branch: bool,
    format: OutputFormat,
    debug: bool,
) -> AppResult<()> {
    let runner = ProcessRunner { debug };
    upgrade_with(&runner, dir, to, dry_run, no_branch, format)
}

/// The renderer: run the upgrade, then emit the `{ok,data|error}` envelope in
/// JSON mode or a human summary. Split from [`run`] so the logic stays testable
/// without capturing stdout.
fn upgrade_with(
    runner: &dyn Runner,
    dir: &Path,
    to: Option<&str>,
    dry_run: bool,
    no_branch: bool,
    format: OutputFormat,
) -> AppResult<()> {
    match run(runner, dir, to, dry_run, no_branch) {
        Ok(report) => emit_report(format, &report),
        Err(refusal) => Err(refuse(format, refusal.kind(), refusal.into_message())),
    }
}

// --- the offline core -------------------------------------------------------

/// The upgrade, minus rendering. Every step that would touch git/npm/node goes
/// through `runner`, so a fake runner makes the whole flow deterministic and
/// offline. A completed run with a failing verify is still an `Ok(report)` whose
/// `verify` is `Fail`; [`emit_report`] maps that to exit 1. Only preconditions
/// that stop the upgrade before it starts are [`Refusal`]s.
fn run(
    runner: &dyn Runner,
    dir: &Path,
    to: Option<&str>,
    dry_run: bool,
    no_branch: bool,
) -> Result<UpgradeReport, Refusal> {
    let manifest_text = read_stamped(dir, "template.toml")?;
    let manifest = parse_manifest(&manifest_text)?;

    let package_text = read_stamped(dir, "package.json")?;
    let package: Value = serde_json::from_str(&package_text)
        .map_err(|e| Refusal::Io(format!("package.json is not valid JSON: {e}")))?;

    let chassis = discover_chassis(&package, &manifest.requires)?;

    // Preflight (spec 006 §2.1): a real run refuses on a dirty tree, checked
    // before target resolution so a dirty tree fails fast with no wasted
    // registry read and the eventual commit is exactly the pin bump and its
    // effects. A dry run mutates nothing, so it does not require a clean tree.
    if !dry_run
        && !runner
            .working_tree_clean(dir)
            .map_err(|e| Refusal::Io(format!("could not read the git working tree: {e}")))?
    {
        return Err(Refusal::DirtyTree(
            "refusing to upgrade a dirty working tree; commit or stash your changes first"
                .to_string(),
        ));
    }

    // Resolve the target: an explicit `--to`, or the greatest published version
    // the primary chassis package's range allows (a registry read, behind Runner).
    let target = match to {
        Some(raw) => Version::parse(raw.trim_start_matches('v')).map_err(|e| {
            Refusal::BadTarget(format!("`--to {raw}` is not a semver version: {e}"))
        })?,
        None => {
            let range = manifest
                .requires
                .get(&chassis.range_key)
                .cloned()
                .unwrap_or_default();
            let req = VersionReq::parse(&range).map_err(|e| {
                Refusal::Io(format!(
                    "contract `[requires].{}` is not a semver range: {e}",
                    chassis.range_key
                ))
            })?;
            runner
                .latest_version(dir, &chassis.pins[chassis.primary].name, &req)
                .map_err(|e| {
                    Refusal::Io(format!(
                        "could not resolve the latest compatible version: {e}"
                    ))
                })?
        }
    };

    // The compat gate is the contract's own ranges: the target must satisfy
    // EVERY chassis requirement, not just the primary's, so a contract that
    // names more than one range can never be crossed silently. Crossing any of
    // them is a migration, not an upgrade, so this verb points at that path.
    for key in &chassis.range_keys {
        let range = manifest.requires.get(key).cloned().unwrap_or_default();
        let req = VersionReq::parse(&range).map_err(|e| {
            Refusal::Io(format!(
                "contract `[requires].{key}` is not a semver range: {e}"
            ))
        })?;
        if !req.matches(&target) {
            return Err(Refusal::IncompatibleTarget(format!(
                "target {target} is outside the contract's compatible range `{range}` for `{key}`; a major upgrade requires the migration path, not this verb"
            )));
        }
    }

    let from = chassis.pins[chassis.primary].current.clone();
    let target_str = target.to_string();
    let pin_reports: Vec<PinReport> = chassis
        .pins
        .iter()
        .map(|p| PinReport {
            name: p.name.clone(),
            from: p.current.clone(),
            to: target_str.clone(),
            section: p.section.to_string(),
        })
        .collect();

    // Already there: an idempotent no-op only when EVERY chassis pin already
    // equals the target, so a companion a partial bump left behind is still
    // caught rather than hidden by the primary already matching.
    if chassis.pins.iter().all(|p| p.current == target_str) {
        return Ok(UpgradeReport {
            from,
            to: target_str,
            pins: Vec::new(),
            codemods_run: Vec::new(),
            verify: VerifyState::Skipped,
            branch: None,
            dry_run,
            pr_suggestion: None,
            note: Some("already at the requested version; nothing to upgrade".to_string()),
        });
    }

    let updated_package =
        apply_pins(&package_text, &chassis.pins, &target_str).map_err(Refusal::Io)?;

    // Dry run: report the plan, mutate nothing, run nothing.
    if dry_run {
        return Ok(UpgradeReport {
            from,
            to: target_str,
            pins: pin_reports,
            codemods_run: Vec::new(),
            verify: VerifyState::Skipped,
            branch: None,
            dry_run: true,
            pr_suggestion: None,
            note: Some("dry run: nothing was changed".to_string()),
        });
    }

    // A branch first, so a failing verify leaves an inspectable branch.
    let branch = if no_branch {
        None
    } else {
        let name = format!("template-upgrade/{from}-{target_str}");
        runner
            .create_branch(dir, &name)
            .map_err(|e| Refusal::Io(format!("could not create branch `{name}`: {e}")))?;
        Some(name)
    };

    std::fs::write(dir.join("package.json"), &updated_package)
        .map_err(|e| Refusal::Io(format!("could not write package.json: {e}")))?;

    runner
        .refresh_lockfile(dir)
        .map_err(|e| Refusal::Io(format!("could not refresh the lockfile: {e}")))?;

    // Codemods are a reserved, template-owned hook (spec 006 §2.3). No template
    // ships them yet, so this is an empty list in practice; the execution path
    // exists so a future template can carry ordered, idempotent codemods.
    let mut codemods_run = Vec::new();
    if let Some(upgrade) = &manifest.upgrade {
        for command in &upgrade.codemods {
            runner
                .run_codemod(dir, command)
                .map_err(|e| Refusal::Io(format!("codemod `{command}` failed: {e}")))?;
            codemods_run.push(command.clone());
        }
    }

    // Verify: the contract's own verb decides the verdict. A failure leaves the
    // branch (with uncommitted changes) for inspection and exits 1 uncommitted.
    let verify = match &manifest.verbs.verify {
        Some(command) => {
            let passed = runner
                .run_verify(dir, command)
                .map_err(|e| Refusal::Io(format!("could not run the verify verb: {e}")))?;
            if passed {
                VerifyState::Pass
            } else {
                VerifyState::Fail
            }
        }
        None => VerifyState::Skipped,
    };

    if verify == VerifyState::Fail {
        return Ok(UpgradeReport {
            from,
            to: target_str,
            pins: pin_reports,
            codemods_run,
            verify,
            branch,
            dry_run: false,
            pr_suggestion: None,
            note: Some("verify failed at step `verify`; the branch is left with uncommitted changes for inspection".to_string()),
        });
    }

    // Commit the bump on the branch with a conventional, spec-referencing message.
    let message = commit_message(&from, &target_str, &pin_reports);
    runner
        .commit(dir, &message)
        .map_err(|e| Refusal::Io(format!("could not commit the upgrade: {e}")))?;

    Ok(UpgradeReport {
        from,
        to: target_str,
        pins: pin_reports,
        codemods_run,
        verify,
        branch,
        dry_run: false,
        pr_suggestion: Some("gh pr create --fill".to_string()),
        note: None,
    })
}

// --- template.toml (the contract surface we consume) ------------------------

/// The subset of `template.toml` (spec 009 / enrahitu 018) the upgrade reads:
/// the chassis version ranges (`[requires]`), the verify verb (`[verbs]`), and
/// the reserved codemod hook (`[upgrade]`). Everything else on the contract is
/// deliberately not this verb's business (spec 006 §1).
#[derive(Debug, Default, Deserialize)]
struct TemplateManifest {
    #[serde(default)]
    requires: BTreeMap<String, String>,
    #[serde(default)]
    verbs: VerbsTable,
    #[serde(default)]
    upgrade: Option<UpgradeTable>,
}

#[derive(Debug, Default, Deserialize)]
struct VerbsTable {
    #[serde(default)]
    verify: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct UpgradeTable {
    /// Ordered, idempotent codemod commands the target template ships. Reserved:
    /// no template ships these yet (spec 006 §2.3).
    #[serde(default)]
    codemods: Vec<String>,
}

fn parse_manifest(text: &str) -> Result<TemplateManifest, Refusal> {
    toml::from_str(text).map_err(|e| Refusal::Io(format!("template.toml is malformed: {e}")))
}

// --- chassis discovery ------------------------------------------------------

/// One chassis dependency the upgrade will re-pin: its full package name, which
/// `package.json` section it lives in, and its current exact pin.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChassisPin {
    name: String,
    section: &'static str,
    current: String,
}

/// The discovered chassis: the pins to bump, the index of the primary seed
/// (whose range the `--to`-omitted registry lookup and the `from` report use),
/// and every distinct `[requires]` key a seed matched. The target must satisfy
/// all of `range_keys`, so a contract naming more than one range can never be
/// crossed silently.
#[derive(Debug)]
struct Chassis {
    pins: Vec<ChassisPin>,
    primary: usize,
    range_key: String,
    range_keys: Vec<String>,
}

const SECTIONS: [&str; 2] = ["dependencies", "devDependencies"];

/// The unscoped tail of a package name: `@enrahitu/toolchain` -> `toolchain`.
fn unscoped(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// The scope of a package name, including the leading `@`: `@enrahitu/toolchain`
/// -> `Some("@enrahitu")`; an unscoped name has no scope.
fn scope_of(name: &str) -> Option<&str> {
    name.strip_prefix('@')
        .map(|_| name.split('/').next().unwrap_or(name))
}

/// True when a version specifier is an exact published-version pin (`0.1.0`),
/// as opposed to a range (`^0.1`), a local link (`file:…`, `link:…`,
/// `workspace:…`), or a git/url spec. Only exact pins are ours to bump.
fn is_exact_pin(spec: &str) -> bool {
    Version::parse(spec.trim()).is_ok()
}

/// Discover the chassis packages to bump. Seeds are the `package.json`
/// dependencies whose unscoped name matches a `[requires]` key; the chassis set
/// is every exact-pinned dependency sharing a seed's scope (so companions ride
/// along). No chassis at all is a pre-018 app; chassis present but only as local
/// links is a template-development checkout, not a stamped app.
fn discover_chassis(
    package: &Value,
    requires: &BTreeMap<String, String>,
) -> Result<Chassis, Refusal> {
    // (name, spec, section) for every declared dependency.
    let mut deps: Vec<(String, String, &'static str)> = Vec::new();
    for section in SECTIONS {
        if let Some(map) = package.get(section).and_then(Value::as_object) {
            for (name, spec) in map {
                if let Some(spec) = spec.as_str() {
                    deps.push((name.clone(), spec.to_string(), section));
                }
            }
        }
    }

    // Seeds: deps that a `[requires]` key names by unscoped tail. `node` and
    // other engine keys match no dependency, so they drop out here.
    let mut seed_keys: BTreeMap<String, String> = BTreeMap::new(); // dep name -> requires key
    for (name, _, _) in &deps {
        let tail = unscoped(name);
        if requires.contains_key(tail) {
            seed_keys.insert(name.clone(), tail.to_string());
        }
    }
    if seed_keys.is_empty() {
        return Err(Refusal::PreChassis(
            "this app predates the packaged chassis: no chassis packages in package.json. See the manual re-import path in the template docs".to_string(),
        ));
    }

    // Scopes owned by the chassis, learned from the seeds (never hardcoded).
    let scopes: Vec<String> = seed_keys
        .keys()
        .filter_map(|n| scope_of(n).map(str::to_string))
        .collect();

    // Candidates: every seed, plus every dependency sharing a chassis scope.
    let pins: Vec<ChassisPin> = deps
        .iter()
        .filter(|(name, spec, _)| {
            let in_scope = scope_of(name).is_some_and(|s| scopes.iter().any(|k| k == s));
            (seed_keys.contains_key(name) || in_scope) && is_exact_pin(spec)
        })
        .map(|(name, spec, section)| ChassisPin {
            name: name.clone(),
            section,
            current: spec.trim().to_string(),
        })
        .collect();

    if pins.is_empty() {
        return Err(Refusal::LocalChassis(
            "the chassis packages are present but not pinned to published versions (a local `file:`/`link:` checkout); this looks like a template-development tree, not a stamped app".to_string(),
        ));
    }

    // The primary seed is the one whose range drives the `--to`-omitted registry
    // lookup and the `from` report: prefer a seed that is itself an exact pin,
    // else fall back to the first exact pin discovered.
    let primary = pins
        .iter()
        .position(|p| seed_keys.contains_key(&p.name))
        .unwrap_or(0);
    let range_key = seed_keys
        .get(&pins[primary].name)
        .cloned()
        .or_else(|| seed_keys.values().next().cloned())
        .unwrap_or_default();
    // Every distinct requirement key a seed matched: the target is gated against
    // all of them, not just the primary's (so multiple ranges cannot be crossed
    // silently). `seed_keys` is a BTreeMap, so this is sorted and deduplicated.
    let mut range_keys: Vec<String> = seed_keys.values().cloned().collect();
    range_keys.dedup();

    Ok(Chassis {
        pins,
        primary,
        range_key,
        range_keys,
    })
}

// --- format-preserving pin rewrite ------------------------------------------

/// Re-pin every chassis dependency to `target` in the original `package.json`
/// text. String-level so key order, indentation, and trailing bytes survive
/// untouched: the git diff is exactly the version bumps.
fn apply_pins(text: &str, pins: &[ChassisPin], target: &str) -> Result<String, String> {
    let mut out = text.to_string();
    for pin in pins {
        out = set_pin(&out, pin.section, &pin.name, target)?;
    }
    Ok(out)
}

/// Replace the version string of `"name"` inside the object introduced by the
/// first `"section"` key. Only the value's inner characters change.
fn set_pin(text: &str, section: &str, name: &str, new_version: &str) -> Result<String, String> {
    let sec_key = format!("\"{section}\"");
    let sec_pos = text
        .find(&sec_key)
        .ok_or_else(|| format!("`{section}` object not found in package.json"))?;
    let name_key = format!("\"{name}\"");
    let name_pos = text[sec_pos..]
        .find(&name_key)
        .map(|i| sec_pos + i)
        .ok_or_else(|| format!("`{name}` not found under `{section}`"))?;
    let after_name = name_pos + name_key.len();
    let colon = text[after_name..]
        .find(':')
        .map(|i| after_name + i + 1)
        .ok_or_else(|| format!("malformed entry for `{name}`"))?;
    let val_open = text[colon..]
        .find('"')
        .map(|i| colon + i + 1)
        .ok_or_else(|| format!("`{name}` value is not a string"))?;
    let val_close = text[val_open..]
        .find('"')
        .map(|i| val_open + i)
        .ok_or_else(|| format!("unterminated value for `{name}`"))?;
    let mut out = String::with_capacity(text.len() + new_version.len());
    out.push_str(&text[..val_open]);
    out.push_str(new_version);
    out.push_str(&text[val_close..]);
    Ok(out)
}

/// The conventional, spec-referencing commit body for the bump.
fn commit_message(from: &str, to: &str, pins: &[PinReport]) -> String {
    use std::fmt::Write;
    let mut msg = format!("chore(template): upgrade chassis {from} -> {to}\n\nPins bumped:\n");
    for p in pins {
        let _ = writeln!(msg, "- {}: {} -> {} ({})", p.name, p.from, p.to, p.section);
    }
    let _ = write!(msg, "\nSpec: 006-template-upgrade-verb");
    msg
}

// --- the result envelope ----------------------------------------------------

/// The verify verdict, serialized into the result (`verify: pass|fail|skipped`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum VerifyState {
    Pass,
    Fail,
    Skipped,
}

/// The upgrade result (spec 006 §2.6): `{from, to, pins, codemodsRun, verify}`
/// plus the branch and PR suggestion. This is the `data` payload of the shared
/// `{ok,data}` envelope, so the platform (and a later fleet-wide sweep) can
/// consume an upgrade the same way both faces already consume every other verb.
#[derive(Debug, Serialize)]
struct UpgradeReport {
    from: String,
    to: String,
    pins: Vec<PinReport>,
    #[serde(rename = "codemodsRun")]
    codemods_run: Vec<String>,
    verify: VerifyState,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(rename = "dryRun", skip_serializing_if = "is_false")]
    dry_run: bool,
    #[serde(rename = "prSuggestion", skip_serializing_if = "Option::is_none")]
    pr_suggestion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct PinReport {
    name: String,
    from: String,
    to: String,
    section: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The `{ok:true,data}` envelope for a completed run. A verify failure is still
/// a well-formed *result* (the run finished and reports its verdict), so it uses
/// the same success envelope, exactly as `stamp status --watch` renders a
/// terminal `failed` job (`emit_ok` then exit 1). The failed verdict rides in
/// `data.verify`; the exit code, not a third envelope shape, carries the
/// failure. Keeping `ok:true` here preserves the two-shape contract the whole
/// codebase reads by (`ok:true` -> `data`, `ok:false` -> `error`).
fn report_envelope(report: &UpgradeReport) -> Value {
    let data = serde_json::to_value(report).expect("serializing an owned report cannot fail");
    super::success_envelope_value(&data)
}

/// Emit the result: the `{ok:true,data}` envelope in JSON, a short summary on a
/// TTY. A verify failure prints the full result (so the platform sees
/// `verify:"fail"` in `data`) and then exits 1, leaving the branch for
/// inspection.
fn emit_report(format: OutputFormat, report: &UpgradeReport) -> AppResult<()> {
    let failed = report.verify == VerifyState::Fail;
    match format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&report_envelope(report))
                .expect("serializing an owned envelope cannot fail")
        ),
        OutputFormat::Human => println!("{}", render_human(report)),
    }
    if failed {
        // The result is already on stdout; signal exit 1 without doubling it. In
        // human mode main adds a stderr line naming the failing step.
        return Err(match format {
            OutputFormat::Json => AppError::Rendered {
                code: EXIT_OPERATIONAL,
            },
            OutputFormat::Human => AppError::Operational(anyhow::anyhow!(
                "verify failed at step `verify`; the upgrade branch is left for inspection"
            )),
        });
    }
    Ok(())
}

fn render_human(report: &UpgradeReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let header = if report.dry_run {
        "template upgrade (dry run)"
    } else {
        "template upgrade"
    };
    let _ = writeln!(out, "{header}: chassis {} -> {}", report.from, report.to);
    if report.pins.is_empty() {
        let _ = writeln!(out, "  (no pins to change)");
    }
    for p in &report.pins {
        let _ = writeln!(out, "  {}: {} -> {}  ({})", p.name, p.from, p.to, p.section);
    }
    if let Some(branch) = &report.branch {
        let _ = writeln!(out, "  branch: {branch}");
    }
    if report.codemods_run.is_empty() {
        let _ = writeln!(out, "  codemods: none");
    } else {
        let _ = writeln!(out, "  codemods: {}", report.codemods_run.join(", "));
    }
    let _ = writeln!(
        out,
        "  verify: {}",
        match report.verify {
            VerifyState::Pass => "pass",
            VerifyState::Fail => "fail",
            VerifyState::Skipped => "skipped",
        }
    );
    if let Some(pr) = &report.pr_suggestion {
        let _ = writeln!(out, "  next: {pr}");
    }
    if let Some(note) = &report.note {
        let _ = write!(out, "  {note}");
    }
    out.trim_end().to_string()
}

// --- refusals ---------------------------------------------------------------

/// A precondition that stops the upgrade before it starts. Carries a stable
/// `kind` for the JSON error envelope and a human message.
#[derive(Debug)]
enum Refusal {
    NotStamped(String),
    PreChassis(String),
    LocalChassis(String),
    BadTarget(String),
    IncompatibleTarget(String),
    DirtyTree(String),
    Io(String),
}

impl Refusal {
    fn kind(&self) -> &'static str {
        match self {
            Refusal::NotStamped(_) => "not_stamped",
            Refusal::PreChassis(_) => "pre_chassis",
            Refusal::LocalChassis(_) => "local_chassis",
            Refusal::BadTarget(_) => "bad_target",
            Refusal::IncompatibleTarget(_) => "incompatible_target",
            Refusal::DirtyTree(_) => "dirty_tree",
            Refusal::Io(_) => "io",
        }
    }

    fn into_message(self) -> String {
        match self {
            Refusal::NotStamped(m)
            | Refusal::PreChassis(m)
            | Refusal::LocalChassis(m)
            | Refusal::BadTarget(m)
            | Refusal::IncompatibleTarget(m)
            | Refusal::DirtyTree(m)
            | Refusal::Io(m) => m,
        }
    }
}

/// Render a refusal: the `{ok:false,error}` envelope + a `Rendered` exit in JSON
/// mode (so stdout carries the machine error and nothing is doubled on stderr),
/// or the operational error for `main` to print on a TTY. Mirrors the API verbs'
/// `emit_err` but for local, non-API failures.
fn refuse(format: OutputFormat, kind: &'static str, message: String) -> AppError {
    match format {
        OutputFormat::Json => {
            let env = super::error_envelope(kind, message, None);
            println!(
                "{}",
                serde_json::to_string_pretty(&env)
                    .expect("serializing an owned envelope cannot fail")
            );
            AppError::Rendered {
                code: EXIT_OPERATIONAL,
            }
        }
        OutputFormat::Human => AppError::Operational(anyhow::anyhow!("{message}")),
    }
}

/// Read a file that a stamped app must have. A missing file is a "not a stamped
/// app" refusal; any other read error is operational.
fn read_stamped(dir: &Path, name: &str) -> Result<String, Refusal> {
    match std::fs::read_to_string(dir.join(name)) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Refusal::NotStamped(format!(
            "not a stamped app: {name} not found in {}",
            dir.display()
        ))),
        Err(e) => Err(Refusal::Io(format!("could not read {name}: {e}"))),
    }
}

// --- the real runner --------------------------------------------------------

/// The external side effects the upgrade orchestrates. Behind a trait so tests
/// inject a deterministic fake and never shell out.
trait Runner {
    /// `git status --porcelain` empty?
    fn working_tree_clean(&self, dir: &Path) -> anyhow::Result<bool>;
    /// The greatest published version of `pkg` that `req` allows (npm registry).
    fn latest_version(&self, dir: &Path, pkg: &str, req: &VersionReq) -> anyhow::Result<Version>;
    /// `git checkout -b <branch>`
    fn create_branch(&self, dir: &Path, branch: &str) -> anyhow::Result<()>;
    /// `npm install --package-lock-only` (never a full, platform-pruning install).
    fn refresh_lockfile(&self, dir: &Path) -> anyhow::Result<()>;
    /// Run one codemod command in the app dir.
    fn run_codemod(&self, dir: &Path, command: &str) -> anyhow::Result<()>;
    /// Run the verify command; `Ok(true)` passed, `Ok(false)` failed.
    fn run_verify(&self, dir: &Path, command: &str) -> anyhow::Result<bool>;
    /// `git add -A && git commit -m <message>`
    fn commit(&self, dir: &Path, message: &str) -> anyhow::Result<()>;
}

/// The production runner: shells out to git, npm, and the platform shell. The
/// only place this verb reaches outside its own process.
struct ProcessRunner {
    debug: bool,
}

impl ProcessRunner {
    fn trace(&self, what: &str) {
        if self.debug {
            eprintln!("template upgrade: {what}");
        }
    }
}

impl Runner for ProcessRunner {
    fn working_tree_clean(&self, dir: &Path) -> anyhow::Result<bool> {
        self.trace("git status --porcelain");
        let out = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(dir)
            .output()?;
        if !out.status.success() {
            anyhow::bail!(
                "git status failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(out.stdout.iter().all(u8::is_ascii_whitespace))
    }

    fn latest_version(&self, dir: &Path, pkg: &str, req: &VersionReq) -> anyhow::Result<Version> {
        self.trace(&format!("npm view {pkg} versions --json"));
        let out = std::process::Command::new("npm")
            .args(["view", pkg, "versions", "--json"])
            .current_dir(dir)
            .output()?;
        if !out.status.success() {
            anyhow::bail!(
                "npm view {pkg} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let versions: Vec<String> = serde_json::from_slice(&out.stdout)
            .map_err(|e| anyhow::anyhow!("npm returned versions we could not parse: {e}"))?;
        versions
            .iter()
            .filter_map(|v| Version::parse(v).ok())
            .filter(|v| req.matches(v))
            .max()
            .ok_or_else(|| anyhow::anyhow!("no published version of {pkg} satisfies `{req}`"))
    }

    fn create_branch(&self, dir: &Path, branch: &str) -> anyhow::Result<()> {
        self.trace(&format!("git checkout -b {branch}"));
        run_ok(
            std::process::Command::new("git")
                .args(["checkout", "-b", branch])
                .current_dir(dir),
        )
    }

    fn refresh_lockfile(&self, dir: &Path) -> anyhow::Result<()> {
        self.trace("npm install --package-lock-only");
        run_ok(
            std::process::Command::new("npm")
                .args(["install", "--package-lock-only"])
                .current_dir(dir),
        )
    }

    fn run_codemod(&self, dir: &Path, command: &str) -> anyhow::Result<()> {
        self.trace(&format!("codemod: {command}"));
        run_ok(shell(command).current_dir(dir))
    }

    fn run_verify(&self, dir: &Path, command: &str) -> anyhow::Result<bool> {
        self.trace(&format!("verify: {command}"));
        Ok(shell(command).current_dir(dir).status()?.success())
    }

    fn commit(&self, dir: &Path, message: &str) -> anyhow::Result<()> {
        self.trace("git add -A && git commit");
        run_ok(
            std::process::Command::new("git")
                .args(["add", "-A"])
                .current_dir(dir),
        )?;
        run_ok(
            std::process::Command::new("git")
                .args(["commit", "-m", message])
                .current_dir(dir),
        )
    }
}

/// Run a command, turning a non-zero exit into an error carrying stderr.
fn run_ok(command: &mut std::process::Command) -> anyhow::Result<()> {
    let out = command.output()?;
    if out.status.success() {
        Ok(())
    } else {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim())
    }
}

/// A shell that runs a command string, so template-owned verbs like
/// `npm run typecheck && npm test` execute as written.
#[cfg(not(target_os = "windows"))]
fn shell(command: &str) -> std::process::Command {
    let mut c = std::process::Command::new("sh");
    c.arg("-c").arg(command);
    c
}

#[cfg(target_os = "windows")]
fn shell(command: &str) -> std::process::Command {
    let mut c = std::process::Command::new("cmd");
    c.args(["/C", command]);
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    fn manifest(requires: &str, verify: Option<&str>) -> String {
        let verify = verify
            .map(|v| format!("[verbs]\nverify = \"{v}\"\n"))
            .unwrap_or_default();
        format!("[template]\nname = \"enrahitu\"\nversion = \"0.1.0\"\n\n[requires]\n{requires}\n\n{verify}")
    }

    fn package(deps: &str, dev: &str) -> String {
        format!("{{\n  \"name\": \"app\",\n  \"dependencies\": {{\n{deps}\n  }},\n  \"devDependencies\": {{\n{dev}\n  }}\n}}\n")
    }

    /// A runner that records what it was asked to do and never touches the disk.
    #[derive(Default)]
    struct FakeRunner {
        clean: bool,
        verify_pass: bool,
        latest: Option<Version>,
        log: RefCell<Vec<String>>,
    }

    impl Runner for FakeRunner {
        fn working_tree_clean(&self, _dir: &Path) -> anyhow::Result<bool> {
            Ok(self.clean)
        }
        fn latest_version(
            &self,
            _dir: &Path,
            _pkg: &str,
            _req: &VersionReq,
        ) -> anyhow::Result<Version> {
            self.latest
                .clone()
                .ok_or_else(|| anyhow::anyhow!("no latest configured"))
        }
        fn create_branch(&self, _dir: &Path, branch: &str) -> anyhow::Result<()> {
            self.log.borrow_mut().push(format!("branch {branch}"));
            Ok(())
        }
        fn refresh_lockfile(&self, _dir: &Path) -> anyhow::Result<()> {
            self.log.borrow_mut().push("lock".to_string());
            Ok(())
        }
        fn run_codemod(&self, _dir: &Path, command: &str) -> anyhow::Result<()> {
            self.log.borrow_mut().push(format!("codemod {command}"));
            Ok(())
        }
        fn run_verify(&self, _dir: &Path, _command: &str) -> anyhow::Result<bool> {
            self.log.borrow_mut().push("verify".to_string());
            Ok(self.verify_pass)
        }
        fn commit(&self, _dir: &Path, _message: &str) -> anyhow::Result<()> {
            self.log.borrow_mut().push("commit".to_string());
            Ok(())
        }
    }

    fn parsed(pkg: &str) -> Value {
        serde_json::from_str(pkg).unwrap()
    }

    #[test]
    fn discovers_scoped_chassis_and_companions() {
        let requires: BTreeMap<String, String> = [
            ("node".into(), ">=24".into()),
            ("toolchain".into(), "^0.1".into()),
        ]
        .into_iter()
        .collect();
        let pkg = parsed(&package(
            "    \"@enrahitu/hiqlite-native\": \"0.1.0\",\n    \"left-pad\": \"^1.0.0\"",
            "    \"@enrahitu/toolchain\": \"0.1.0\"",
        ));
        let chassis = discover_chassis(&pkg, &requires).unwrap();
        let names: Vec<&str> = chassis.pins.iter().map(|p| p.name.as_str()).collect();
        // The seed and its scope companion are bumped; a third-party dep is not.
        assert!(names.contains(&"@enrahitu/toolchain"));
        assert!(names.contains(&"@enrahitu/hiqlite-native"));
        assert!(!names.contains(&"left-pad"));
        assert_eq!(chassis.range_key, "toolchain");
        assert_eq!(chassis.pins[chassis.primary].name, "@enrahitu/toolchain");
    }

    #[test]
    fn no_chassis_is_a_pre_018_refusal() {
        let requires: BTreeMap<String, String> =
            [("toolchain".into(), "^0.1".into())].into_iter().collect();
        let pkg = parsed(&package(
            "    \"left-pad\": \"^1.0.0\"",
            "    \"vitest\": \"^1.0.0\"",
        ));
        let err = discover_chassis(&pkg, &requires).unwrap_err();
        assert_eq!(err.kind(), "pre_chassis");
    }

    #[test]
    fn locally_linked_chassis_is_a_dev_tree_refusal() {
        let requires: BTreeMap<String, String> =
            [("toolchain".into(), "^0.1".into())].into_iter().collect();
        // The chassis is present but as a file: link, as in the template repo.
        let pkg = parsed(&package(
            "    \"@enrahitu/hiqlite-native\": \"file:./addon\"",
            "    \"@enrahitu/toolchain\": \"file:./packages/toolchain\"",
        ));
        let err = discover_chassis(&pkg, &requires).unwrap_err();
        assert_eq!(err.kind(), "local_chassis");
    }

    #[test]
    fn set_pin_preserves_surrounding_text() {
        let text = package(
            "    \"@enrahitu/hiqlite-native\": \"0.1.0\",\n    \"left-pad\": \"1.0.0\"",
            "    \"@enrahitu/toolchain\": \"0.1.0\"",
        );
        let out = set_pin(&text, "devDependencies", "@enrahitu/toolchain", "0.2.0").unwrap();
        assert!(out.contains("\"@enrahitu/toolchain\": \"0.2.0\""));
        // The unrelated dependency and its pin are untouched.
        assert!(out.contains("\"left-pad\": \"1.0.0\""));
        assert!(out.contains("\"@enrahitu/hiqlite-native\": \"0.1.0\""));
    }

    #[test]
    fn apply_pins_bumps_every_chassis_dep() {
        let requires: BTreeMap<String, String> =
            [("toolchain".into(), "^0.1".into())].into_iter().collect();
        let text = package(
            "    \"@enrahitu/hiqlite-native\": \"0.1.0\"",
            "    \"@enrahitu/toolchain\": \"0.1.0\"",
        );
        let chassis = discover_chassis(&parsed(&text), &requires).unwrap();
        let out = apply_pins(&text, &chassis.pins, "0.1.5").unwrap();
        assert!(out.contains("\"@enrahitu/toolchain\": \"0.1.5\""));
        assert!(out.contains("\"@enrahitu/hiqlite-native\": \"0.1.5\""));
    }

    #[test]
    fn dry_run_reports_the_plan_and_touches_nothing() {
        let dir = tempdir();
        std::fs::write(
            dir.join("template.toml"),
            manifest("toolchain = \"^0.1\"", Some("npm test")),
        )
        .unwrap();
        let pkg = package("", "    \"@enrahitu/toolchain\": \"0.1.0\"");
        std::fs::write(dir.join("package.json"), &pkg).unwrap();

        let runner = FakeRunner::default();
        let report = run(&runner, &dir, Some("0.1.5"), true, false).unwrap();

        assert!(report.dry_run);
        assert_eq!(report.to, "0.1.5");
        assert_eq!(report.verify, VerifyState::Skipped);
        // Nothing ran and the file on disk is byte-for-byte unchanged.
        assert!(runner.log.borrow().is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.join("package.json")).unwrap(),
            pkg
        );
    }

    #[test]
    fn incompatible_target_refuses_as_a_migration() {
        let dir = tempdir();
        std::fs::write(
            dir.join("template.toml"),
            manifest("toolchain = \"^0.1\"", Some("npm test")),
        )
        .unwrap();
        std::fs::write(
            dir.join("package.json"),
            package("", "    \"@enrahitu/toolchain\": \"0.1.0\""),
        )
        .unwrap();

        // A clean tree, so the run reaches the compat gate rather than stopping
        // at the dirty-tree preflight that now precedes target resolution.
        let runner = FakeRunner {
            clean: true,
            ..Default::default()
        };
        let err = run(&runner, &dir, Some("0.2.0"), false, false).unwrap_err();
        assert_eq!(err.kind(), "incompatible_target");
    }

    #[test]
    fn dirty_tree_refuses_before_mutating() {
        let dir = tempdir();
        std::fs::write(
            dir.join("template.toml"),
            manifest("toolchain = \"^0.1\"", Some("npm test")),
        )
        .unwrap();
        let pkg = package("", "    \"@enrahitu/toolchain\": \"0.1.0\"");
        std::fs::write(dir.join("package.json"), &pkg).unwrap();

        let runner = FakeRunner {
            clean: false,
            ..Default::default()
        };
        let err = run(&runner, &dir, Some("0.1.5"), false, false).unwrap_err();
        assert_eq!(err.kind(), "dirty_tree");
        // Refused before any write: package.json is untouched.
        assert_eq!(
            std::fs::read_to_string(dir.join("package.json")).unwrap(),
            pkg
        );
    }

    #[test]
    fn happy_path_bumps_branches_and_commits() {
        let dir = tempdir();
        std::fs::write(
            dir.join("template.toml"),
            manifest("toolchain = \"^0.1\"", Some("npm test")),
        )
        .unwrap();
        std::fs::write(
            dir.join("package.json"),
            package("", "    \"@enrahitu/toolchain\": \"0.1.0\""),
        )
        .unwrap();

        let runner = FakeRunner {
            clean: true,
            verify_pass: true,
            ..Default::default()
        };
        let report = run(&runner, &dir, Some("0.1.5"), false, false).unwrap();

        assert_eq!(report.verify, VerifyState::Pass);
        assert_eq!(
            report.branch.as_deref(),
            Some("template-upgrade/0.1.0-0.1.5")
        );
        assert_eq!(report.pr_suggestion.as_deref(), Some("gh pr create --fill"));
        let log = runner.log.borrow();
        assert_eq!(
            *log,
            vec![
                "branch template-upgrade/0.1.0-0.1.5",
                "lock",
                "verify",
                "commit"
            ]
        );
        // The bump is on disk.
        let written = std::fs::read_to_string(dir.join("package.json")).unwrap();
        assert!(written.contains("\"@enrahitu/toolchain\": \"0.1.5\""));
    }

    #[test]
    fn verify_failure_leaves_the_branch_uncommitted() {
        let dir = tempdir();
        std::fs::write(
            dir.join("template.toml"),
            manifest("toolchain = \"^0.1\"", Some("npm test")),
        )
        .unwrap();
        std::fs::write(
            dir.join("package.json"),
            package("", "    \"@enrahitu/toolchain\": \"0.1.0\""),
        )
        .unwrap();

        let runner = FakeRunner {
            clean: true,
            verify_pass: false,
            ..Default::default()
        };
        let report = run(&runner, &dir, Some("0.1.5"), false, false).unwrap();

        assert_eq!(report.verify, VerifyState::Fail);
        // Branch created, changes written, but no commit.
        let log = runner.log.borrow();
        assert!(log.contains(&"branch template-upgrade/0.1.0-0.1.5".to_string()));
        assert!(log.contains(&"verify".to_string()));
        assert!(!log.contains(&"commit".to_string()));
    }

    #[test]
    fn resolves_latest_when_to_is_omitted() {
        let dir = tempdir();
        std::fs::write(
            dir.join("template.toml"),
            manifest("toolchain = \"^0.1\"", Some("npm test")),
        )
        .unwrap();
        std::fs::write(
            dir.join("package.json"),
            package("", "    \"@enrahitu/toolchain\": \"0.1.0\""),
        )
        .unwrap();

        let runner = FakeRunner {
            clean: true,
            verify_pass: true,
            latest: Some(Version::new(0, 1, 9)),
            ..Default::default()
        };
        let report = run(&runner, &dir, None, false, false).unwrap();
        assert_eq!(report.to, "0.1.9");
    }

    #[test]
    fn missing_template_toml_is_not_stamped() {
        let dir = tempdir();
        std::fs::write(dir.join("package.json"), package("", "")).unwrap();
        let err = run(&FakeRunner::default(), &dir, Some("0.1.5"), false, false).unwrap_err();
        assert_eq!(err.kind(), "not_stamped");
    }

    #[test]
    fn success_envelope_snapshot() {
        let report = UpgradeReport {
            from: "0.1.0".to_string(),
            to: "0.1.5".to_string(),
            pins: vec![PinReport {
                name: "@enrahitu/toolchain".to_string(),
                from: "0.1.0".to_string(),
                to: "0.1.5".to_string(),
                section: "devDependencies".to_string(),
            }],
            codemods_run: Vec::new(),
            verify: VerifyState::Pass,
            branch: Some("template-upgrade/0.1.0-0.1.5".to_string()),
            dry_run: false,
            pr_suggestion: Some("gh pr create --fill".to_string()),
            note: None,
        };
        let data = serde_json::to_value(&report).unwrap();
        let env = super::super::success_envelope_value(&data);
        assert_eq!(env["ok"], true);
        assert_eq!(env["data"]["verify"], "pass");
        assert_eq!(env["data"]["codemodsRun"], json!([]));
        assert_eq!(env["data"]["from"], "0.1.0");
        // `dryRun` is omitted when false, keeping the common envelope lean.
        assert!(env["data"].get("dryRun").is_none());
    }

    #[test]
    fn verify_fail_result_stays_ok_true_and_exits_one() {
        // A verify failure is a completed run: `{ok:true, data:{verify:"fail"}}`
        // (the two-shape contract holds), with the failure carried by exit 1 --
        // the same shape `stamp status --watch` uses for a terminal failed job.
        let fail = UpgradeReport {
            from: "0.1.0".to_string(),
            to: "0.1.5".to_string(),
            pins: vec![PinReport {
                name: "@enrahitu/toolchain".to_string(),
                from: "0.1.0".to_string(),
                to: "0.1.5".to_string(),
                section: "devDependencies".to_string(),
            }],
            codemods_run: Vec::new(),
            verify: VerifyState::Fail,
            branch: Some("template-upgrade/0.1.0-0.1.5".to_string()),
            dry_run: false,
            pr_suggestion: None,
            note: None,
        };
        let env = report_envelope(&fail);
        assert_eq!(
            env["ok"], true,
            "a completed run is ok:true even on verify fail"
        );
        assert_eq!(env["data"]["verify"], "fail");
        assert!(
            env.get("error").is_none(),
            "no error arm on a completed run"
        );
        // Rendered exit 1 in JSON mode; the result is already on stdout.
        assert!(matches!(
            emit_report(OutputFormat::Json, &fail),
            Err(AppError::Rendered {
                code: EXIT_OPERATIONAL
            })
        ));

        // A passing run is the same envelope shape but exits 0.
        let mut pass = fail;
        pass.verify = VerifyState::Pass;
        assert_eq!(report_envelope(&pass)["data"]["verify"], "pass");
        assert!(emit_report(OutputFormat::Json, &pass).is_ok());
    }

    #[test]
    fn companion_behind_the_primary_is_not_a_no_op() {
        // The primary is already at the target but a scope companion lags: the
        // no-op check compares every pin, so the companion is still bumped.
        let dir = tempdir();
        std::fs::write(
            dir.join("template.toml"),
            manifest("toolchain = \"^0.1\"", Some("npm test")),
        )
        .unwrap();
        std::fs::write(
            dir.join("package.json"),
            package(
                "    \"@enrahitu/hiqlite-native\": \"0.1.0\"",
                "    \"@enrahitu/toolchain\": \"0.1.5\"",
            ),
        )
        .unwrap();

        let runner = FakeRunner {
            clean: true,
            verify_pass: true,
            ..Default::default()
        };
        let report = run(&runner, &dir, Some("0.1.5"), false, false).unwrap();
        // Not the no-op branch: the run proceeded and verify ran.
        assert_eq!(report.verify, VerifyState::Pass);
        assert!(report
            .pins
            .iter()
            .any(|p| p.name == "@enrahitu/hiqlite-native" && p.from == "0.1.0"));
    }

    #[test]
    fn multi_seed_gate_refuses_when_any_range_excludes_the_target() {
        // Two chassis requirements; the target satisfies one range but not the
        // other. The gate checks every range, so it refuses regardless of which
        // seed happens to be primary.
        let dir = tempdir();
        std::fs::write(
            dir.join("template.toml"),
            manifest(
                "toolchain = \"<0.1.4\"\nhiqlite-native = \"^0.1\"",
                Some("npm test"),
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("package.json"),
            package(
                "    \"@enrahitu/hiqlite-native\": \"0.1.0\"",
                "    \"@enrahitu/toolchain\": \"0.1.0\"",
            ),
        )
        .unwrap();

        let runner = FakeRunner {
            clean: true,
            ..Default::default()
        };
        let err = run(&runner, &dir, Some("0.1.5"), false, false).unwrap_err();
        assert_eq!(err.kind(), "incompatible_target");
    }

    /// A unique temp dir under the OS temp root, no external tempfile crate. The
    /// pid keeps parallel test binaries from colliding; each test uses a fresh
    /// subdir via a counter.
    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("statecraft-tmpl-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
