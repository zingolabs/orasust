#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::{SystemTime, UNIX_EPOCH};

use http::HeaderValue;
use oci_client::client::{Client, ClientConfig, ClientProtocol, Config, ImageLayer};
use oci_client::errors::{OciDistributionError, OciErrorCode};
use oci_client::manifest::IMAGE_MANIFEST_MEDIA_TYPE;
use oci_client::secrets::RegistryAuth;
use oci_client::{Reference, RegistryOperation};
use oci_spec::distribution::Reference as SpecReference;
use oci_spec::image::{ANNOTATION_CREATED, ANNOTATION_TITLE, MediaType};
use serde_json::{Value, json};

#[allow(clippy::all, dead_code)]
mod image_spec {
    include!(concat!(env!("OUT_DIR"), "/image_spec.rs"));
}

use image_spec::{
    ContentDescriptor, DefsBase64, DefsDescriptorAnnotations, DefsDescriptorDigest,
    DefsDescriptorMediaType, DefsDescriptorSize, DefsMapStringString, DefsMapStringStringKey,
    ImageManifest,
};

const NAME: &str = env!("CARGO_PKG_NAME");
const VERSION: &str = env!("CARGO_PKG_VERSION");

const PUSH: &str = "push";
const RESOLVE: &str = "resolve";
const MANIFEST: &str = "manifest";
const FETCH: &str = "fetch";
const VERSION_COMMAND: &str = "version";

const ANNOTATION_FLAG: &str = "--annotation";
const ANNOTATION_SHORT: &str = "-a";
const ARTIFACT_TYPE_FLAG: &str = "--artifact-type";
const FORMAT_FLAG: &str = "--format";
const USERNAME_FLAG: &str = "--username";
const USERNAME_SHORT: &str = "-u";
const PASSWORD_FLAG: &str = "--password";
const PASSWORD_SHORT: &str = "-p";
const PASSWORD_STDIN_FLAG: &str = "--password-stdin";
const PLAIN_HTTP_FLAG: &str = "--plain-http";
const FULL_REFERENCE_FLAG: &str = "--full-reference";
const FULL_REFERENCE_SHORT: &str = "-l";
const DESCRIPTOR_FLAG: &str = "--descriptor";
const PRETTY_FLAG: &str = "--pretty";
const FORMAT_TEXT: &str = "text";
const FORMAT_JSON: &str = "json";
const FLAG_PREFIX: char = '-';
const ANNOTATION_SEPARATOR: char = '=';
const MEDIA_TYPE_SEPARATOR: char = ':';
const MEDIA_TYPE_MARK: char = '/';
const DIGEST_SEPARATOR: char = '@';
const REPOSITORY_SEPARATOR: char = '/';

const DEFAULT_ARTIFACT_TYPE: &str = "application/vnd.unknown.artifact.v1";
const EMPTY_CONFIG: &str = "{}";
const EMPTY_CONFIG_DATA: &str = "e30=";
const SCHEMA_VERSION: i64 = 2;

fn manifest_media_type() -> String {
    MediaType::ImageManifest.to_string()
}

fn empty_media_type() -> String {
    MediaType::EmptyJSON.to_string()
}

fn layer_media_type() -> String {
    MediaType::ImageLayer.to_string()
}
const SHORT_DIGEST_HEX: usize = 12;
const DIGEST_PREFIX: &str = "sha256:";

const PUSHED_TARGET: &str = "[registry]";
const UPLOADED_LABEL: &str = "Uploaded ";
const PUSHED_LABEL: &str = "Pushed";
const ARTIFACT_TYPE_LABEL: &str = "ArtifactType:";
const DIGEST_LABEL: &str = "Digest:";
const VERSION_LABEL: &str = "Version:";
const OS_ARCH_LABEL: &str = "OS/Arch:";
const LABEL_WIDTH: usize = 16;
const ERROR_PREFIX: &str = "Error:";
const REGISTRY_ERROR_PREFIX: &str = "Error response from registry:";
const CREDENTIAL_NOT_FOUND: &str = "basic credential not found";
const NOT_FOUND: &str = "not found";
const UNAUTHORIZED: u16 = 401;
const DISABLE_PATH_VALIDATION_FLAG: &str = "--disable-path-validation";
const CONFIG_KEY: &str = "config";
const SUBJECT_KEY: &str = "subject";
const LAYERS_KEY: &str = "layers";
const MANIFEST_ORDER: [&str; 7] = [
    "schemaVersion",
    "mediaType",
    "artifactType",
    CONFIG_KEY,
    LAYERS_KEY,
    SUBJECT_KEY,
    "annotations",
];
const DESCRIPTOR_ORDER: [&str; 7] = [
    "mediaType",
    "digest",
    "size",
    "urls",
    "annotations",
    "data",
    "artifactType",
];

const USAGE: &str = "Usage:
  orasust push [flags] <name>[:<tag>] <file>[:<type>] [...]
  orasust resolve [flags] <name>{:<tag>|@<digest>}
  orasust manifest fetch [flags] <name>{:<tag>|@<digest>}
  orasust version

Flags:
  -a, --annotation <key>=<value>   manifest annotation (push)
      --artifact-type <type>       artifact type (push)
      --format text|json           output format (push)
  -l, --full-reference             print the full reference with digest (resolve)
      --descriptor                 output the descriptor (manifest fetch)
      --pretty                     prettify JSON printed to stdout (manifest fetch)
      --disable-path-validation    skip path validation (push)
  -u, --username <name>            registry username
  -p, --password <password>        registry password or token
      --password-stdin             read the password from stdin
      --plain-http                 allow insecure connections to the registry";

const EPOCH_SHIFT: i64 = 719_468;
const DAYS_PER_ERA: i64 = 146_097;
const DAYS_PER_4Y: u64 = 1_460;
const DAYS_PER_100Y: u64 = 36_524;
const DAYS_PER_400Y: u64 = 146_096;
const DAYS_PER_YEAR: u64 = 365;
const SECONDS_PER_DAY: u64 = 86_400;
const SECONDS_PER_HOUR: u64 = 3_600;
const SECONDS_PER_MINUTE: u64 = 60;
const YEARS_PER_ERA: i64 = 400;
const MONTHS_BEFORE_MARCH: u64 = 2;
const MONTHS_AFTER_FEBRUARY: u64 = 10;
const MARCH: u64 = 3;
const MONTH_SHIFT: u64 = 9;
const DAY_OF_YEAR_SCALE: u64 = 153;
const DAY_OF_YEAR_OFFSET: u64 = 2;
const DAY_OF_YEAR_DIVISOR: u64 = 5;

#[derive(Debug, PartialEq, Clone, Copy)]
enum Format {
    Text,
    Json,
}

#[derive(Debug, PartialEq)]
struct PushOptions {
    artifact_type: String,
    format: Format,
    disable_path_validation: bool,
}

#[derive(Debug, PartialEq)]
enum Command {
    Push {
        name: String,
        files: Vec<(PathBuf, String)>,
        annotations: Vec<(String, String)>,
        settings: PushOptions,
    },
    Resolve {
        name: String,
        full_reference: bool,
    },
    ManifestFetch {
        name: String,
        descriptor: bool,
        pretty: bool,
    },
    Version,
}

#[derive(Debug, PartialEq, Default)]
struct Options {
    username: Option<String>,
    password: Option<String>,
    password_from_stdin: bool,
    plain_http: bool,
}

#[derive(Debug, PartialEq, Default)]
struct Flags {
    options: Options,
    annotations: Vec<(String, String)>,
    artifact_type: Option<String>,
    format: Option<Format>,
    disable_path_validation: bool,
    full_reference: bool,
    descriptor: bool,
    pretty: bool,
}

#[derive(Debug, PartialEq)]
struct Request {
    options: Options,
    command: Command,
}

fn value_of<'a>(
    iter: &mut impl Iterator<Item = &'a String>,
    flag: &str,
) -> Result<&'a str, String> {
    iter.next()
        .map(String::as_str)
        .ok_or_else(|| format!("flag needs an argument: {flag}"))
}

fn annotation(text: &str) -> Result<(String, String), String> {
    text.split_once(ANNOTATION_SEPARATOR)
        .filter(|(key, _)| !key.is_empty())
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .ok_or_else(|| format!("annotation value doesn't match the required format: {text}"))
}

fn format(text: &str) -> Result<Format, String> {
    match text {
        FORMAT_TEXT => Ok(Format::Text),
        FORMAT_JSON => Ok(Format::Json),
        _ => Err(format!("unsupported format type: {text}")),
    }
}

fn file_and_media_type(word: &str) -> (PathBuf, String) {
    match word.rsplit_once(MEDIA_TYPE_SEPARATOR) {
        Some((file, media_type)) if media_type.contains(MEDIA_TYPE_MARK) => {
            (PathBuf::from(file), media_type.to_string())
        }
        Some((file, "")) => (PathBuf::from(file), layer_media_type()),
        _ => (PathBuf::from(word), layer_media_type()),
    }
}

fn parse(args: &[String]) -> Result<Request, String> {
    let mut flags = Flags::default();
    let mut words: Vec<&str> = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            ANNOTATION_FLAG | ANNOTATION_SHORT => flags
                .annotations
                .push(annotation(value_of(&mut iter, ANNOTATION_FLAG)?)?),
            ARTIFACT_TYPE_FLAG => {
                flags.artifact_type = Some(value_of(&mut iter, ARTIFACT_TYPE_FLAG)?.to_string());
            }
            FORMAT_FLAG => flags.format = Some(format(value_of(&mut iter, FORMAT_FLAG)?)?),
            USERNAME_FLAG | USERNAME_SHORT => {
                flags.options.username = Some(value_of(&mut iter, USERNAME_FLAG)?.to_string());
            }
            PASSWORD_FLAG | PASSWORD_SHORT => {
                flags.options.password = Some(value_of(&mut iter, PASSWORD_FLAG)?.to_string());
            }
            PASSWORD_STDIN_FLAG => flags.options.password_from_stdin = true,
            PLAIN_HTTP_FLAG => flags.options.plain_http = true,
            DISABLE_PATH_VALIDATION_FLAG => flags.disable_path_validation = true,
            FULL_REFERENCE_FLAG | FULL_REFERENCE_SHORT => flags.full_reference = true,
            DESCRIPTOR_FLAG => flags.descriptor = true,
            PRETTY_FLAG => flags.pretty = true,
            flag if flag.starts_with(FLAG_PREFIX) => {
                return Err(format!("unknown flag: {flag}\n{USAGE}"));
            }
            word => words.push(word),
        }
    }
    let Flags {
        options,
        annotations,
        artifact_type,
        format,
        disable_path_validation,
        full_reference,
        descriptor,
        pretty,
    } = flags;
    let push_flags = !annotations.is_empty()
        || artifact_type.is_some()
        || format.is_some()
        || disable_path_validation;
    let resolve_flags = full_reference;
    let fetch_flags = descriptor || pretty;
    let (command, misplaced) = match words.as_slice() {
        [PUSH, name, files @ ..] if !files.is_empty() => (
            Command::Push {
                name: name.to_string(),
                files: files.iter().map(|file| file_and_media_type(file)).collect(),
                annotations,
                settings: PushOptions {
                    artifact_type: artifact_type
                        .unwrap_or_else(|| DEFAULT_ARTIFACT_TYPE.to_string()),
                    format: format.unwrap_or(Format::Text),
                    disable_path_validation,
                },
            },
            resolve_flags || fetch_flags,
        ),
        [RESOLVE, name] => (
            Command::Resolve {
                name: name.to_string(),
                full_reference,
            },
            push_flags || fetch_flags,
        ),
        [MANIFEST, FETCH, name] => (
            Command::ManifestFetch {
                name: name.to_string(),
                descriptor,
                pretty,
            },
            push_flags || resolve_flags,
        ),
        [VERSION_COMMAND] => (
            Command::Version,
            push_flags || resolve_flags || fetch_flags || options != Options::default(),
        ),
        [PUSH, ..] => {
            return Err(format!(
                "\"{NAME} {PUSH}\" requires at least 2 arguments but got {}\n{USAGE}",
                words.len() - 1
            ));
        }
        _ => return Err(USAGE.to_string()),
    };
    if misplaced {
        return Err(format!("a flag does not belong to this command\n{USAGE}"));
    }
    Ok(Request { options, command })
}

fn auth(options: &Options) -> Result<RegistryAuth, String> {
    let password = match (&options.password, options.password_from_stdin) {
        (Some(_), true) => {
            return Err(format!(
                "{PASSWORD_FLAG} and {PASSWORD_STDIN_FLAG} cannot be both used"
            ));
        }
        (Some(password), false) => Some(password.clone()),
        (None, true) => {
            let mut password = String::new();
            std::io::stdin()
                .read_to_string(&mut password)
                .map_err(|e| format!("failed to read password from stdin: {e}"))?;
            Some(password.trim_end_matches(['\r', '\n']).to_string())
        }
        (None, false) => None,
    };
    match (&options.username, password) {
        (None, None) => Ok(RegistryAuth::Anonymous),
        (Some(username), Some(password)) => Ok(RegistryAuth::Basic(username.clone(), password)),
        (Some(_), None) => Err(format!(
            "{USERNAME_FLAG} needs {PASSWORD_FLAG} or {PASSWORD_STDIN_FLAG}"
        )),
        (None, Some(_)) => Err(format!("a password needs {USERNAME_FLAG}")),
    }
}

#[derive(Debug, PartialEq)]
enum Failure {
    Local(String),
    Registry(String),
}

impl From<String> for Failure {
    fn from(diagnostic: String) -> Self {
        Self::Local(diagnostic)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(diagnostic) => write!(f, "{ERROR_PREFIX} {diagnostic}"),
            Self::Registry(diagnostic) => write!(f, "{REGISTRY_ERROR_PREFIX} {diagnostic}"),
        }
    }
}

fn reason_phrase(code: u16) -> &'static str {
    http::StatusCode::from_u16(code)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or_default()
}

fn is_not_found(code: &OciErrorCode) -> bool {
    matches!(
        code,
        OciErrorCode::BlobUnknown
            | OciErrorCode::ManifestBlobUnknown
            | OciErrorCode::ManifestUnknown
            | OciErrorCode::NameUnknown
            | OciErrorCode::NotFound
    )
}

fn registry_failure(
    action: &str,
    name: &str,
    anonymous: bool,
    error: OciDistributionError,
) -> Failure {
    match error {
        OciDistributionError::UnauthorizedError { .. }
        | OciDistributionError::AuthenticationFailure(_)
            if anonymous =>
        {
            Failure::Local(CREDENTIAL_NOT_FOUND.to_string())
        }
        OciDistributionError::ServerError { code, .. } if anonymous && code == UNAUTHORIZED => {
            Failure::Local(CREDENTIAL_NOT_FOUND.to_string())
        }
        OciDistributionError::UnauthorizedError { url } => Failure::Registry(format!(
            "\"{url}\": response status code {UNAUTHORIZED}: {}",
            reason_phrase(UNAUTHORIZED)
        )),
        OciDistributionError::ServerError { url, code, .. } => Failure::Registry(format!(
            "\"{url}\": response status code {code}: {}",
            reason_phrase(code)
        )),
        OciDistributionError::RegistryError { envelope, .. }
            if envelope.errors.iter().all(|e| is_not_found(&e.code)) =>
        {
            Failure::Registry(format!("failed to {action}: {name}: {NOT_FOUND}"))
        }
        OciDistributionError::ImageManifestNotFoundError(_) => {
            Failure::Registry(format!("failed to {action}: {name}: {NOT_FOUND}"))
        }
        OciDistributionError::RegistryError { envelope, .. } => {
            Failure::Registry(format!("failed to {action}: {name}: {envelope}"))
        }
        other => Failure::Local(format!("failed to {action}: {other}")),
    }
}

fn reference(name: &str) -> Result<Reference, String> {
    SpecReference::try_from(name).map_err(|e| format!("invalid reference \"{name}\": {e}"))?;
    Reference::try_from(name).map_err(|e| format!("invalid reference \"{name}\": {e}"))
}

fn repository(image: &Reference) -> String {
    format!(
        "{}{REPOSITORY_SEPARATOR}{}",
        image.registry(),
        image.repository()
    )
}

fn file_name(file: &Path) -> Result<String, String> {
    file.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{} has no file name", file.display()))
}

fn sha256_digest(data: &[u8]) -> String {
    Config::new(data.to_vec(), String::new(), None).sha256_digest()
}

fn short(digest: &str) -> &str {
    let hex = digest.strip_prefix(DIGEST_PREFIX).unwrap_or(digest);
    &hex[..hex.len().min(SHORT_DIGEST_HEX)]
}

fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let shifted = days + EPOCH_SHIFT;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - (DAYS_PER_ERA - 1)
    } / DAYS_PER_ERA;
    let day_of_era = (shifted - era * DAYS_PER_ERA) as u64;
    let year_of_era = (day_of_era - day_of_era / DAYS_PER_4Y + day_of_era / DAYS_PER_100Y
        - day_of_era / DAYS_PER_400Y)
        / DAYS_PER_YEAR;
    let year = year_of_era as i64 + era * YEARS_PER_ERA;
    let day_of_year =
        day_of_era - (DAYS_PER_YEAR * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (DAY_OF_YEAR_DIVISOR * day_of_year + DAY_OF_YEAR_OFFSET) / DAY_OF_YEAR_SCALE;
    let day = day_of_year
        - (DAY_OF_YEAR_SCALE * month_index + DAY_OF_YEAR_OFFSET) / DAY_OF_YEAR_DIVISOR
        + 1;
    let month = if month_index < MONTHS_AFTER_FEBRUARY {
        month_index + MARCH
    } else {
        month_index - MONTH_SHIFT
    };
    (
        if month <= MONTHS_BEFORE_MARCH {
            year + 1
        } else {
            year
        },
        month,
        day,
    )
}

fn rfc3339_utc(seconds: u64) -> String {
    let (year, month, day) = civil_from_days((seconds / SECONDS_PER_DAY) as i64);
    let rest = seconds % SECONDS_PER_DAY;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / SECONDS_PER_HOUR,
        rest % SECONDS_PER_HOUR / SECONDS_PER_MINUTE,
        rest % SECONDS_PER_MINUTE
    )
}

fn created_now() -> Result<String, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| rfc3339_utc(since.as_secs()))
        .map_err(|e| format!("the clock is before the epoch: {e}"))
}

fn media_type(text: &str) -> Result<DefsDescriptorMediaType, String> {
    text.parse()
        .map_err(|e| format!("{text} is not a media type the image spec allows: {e}"))
}

fn digest(text: &str) -> Result<DefsDescriptorDigest, String> {
    text.parse()
        .map_err(|e| format!("{text} is not a digest the image spec allows: {e}"))
}

fn size(length: usize) -> Result<DefsDescriptorSize, String> {
    i64::try_from(length)
        .map(DefsDescriptorSize)
        .map_err(|e| format!("{length} bytes exceed the size the image spec allows: {e}"))
}

fn annotations(
    pairs: impl IntoIterator<Item = (String, String)>,
) -> Result<DefsDescriptorAnnotations, String> {
    let mut map = BTreeMap::new();
    for (key, value) in pairs {
        let key: DefsMapStringStringKey = key
            .parse()
            .map_err(|e| format!("{key} is not an annotation key the image spec allows: {e}"))?;
        map.insert(key, value);
    }
    Ok(DefsDescriptorAnnotations(DefsMapStringString(map)))
}

fn descriptor(
    media_type_text: &str,
    digest_text: &str,
    length: usize,
    annotations: Option<DefsDescriptorAnnotations>,
    inline: Option<&str>,
) -> Result<ContentDescriptor, String> {
    Ok(ContentDescriptor {
        annotations,
        artifact_type: None,
        data: inline.map(|data| DefsBase64(data.to_string())),
        digest: digest(digest_text)?,
        media_type: media_type(media_type_text)?,
        size: size(length)?,
        urls: None,
    })
}

fn manifest(
    artifact_type: &str,
    layers: Vec<ContentDescriptor>,
    manifest_annotations: BTreeMap<String, String>,
) -> Result<ImageManifest, String> {
    Ok(ImageManifest {
        annotations: Some(annotations(manifest_annotations)?),
        artifact_type: Some(media_type(artifact_type)?),
        config: descriptor(
            &empty_media_type(),
            &sha256_digest(EMPTY_CONFIG.as_bytes()),
            EMPTY_CONFIG.len(),
            None,
            Some(EMPTY_CONFIG_DATA),
        )?,
        layers,
        media_type: Some(media_type(&manifest_media_type())?),
        schema_version: SCHEMA_VERSION,
        subject: None,
    })
}

fn descriptor_of(manifest_bytes: &[u8]) -> Result<ContentDescriptor, String> {
    let parsed: ImageManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| format!("the manifest does not match the image spec: {e}"))?;
    Ok(ContentDescriptor {
        annotations: None,
        artifact_type: None,
        data: None,
        digest: digest(&sha256_digest(manifest_bytes))?,
        media_type: match parsed.media_type {
            Some(found) => found,
            None => media_type(&manifest_media_type())?,
        },
        size: size(manifest_bytes.len())?,
        urls: None,
    })
}

fn ordered(value: Value, keys: &[&str]) -> Value {
    match value {
        Value::Object(mut fields) => {
            let named: Vec<(String, Value)> = keys
                .iter()
                .filter_map(|key| fields.remove(*key).map(|field| ((*key).to_string(), field)))
                .collect();
            Value::Object(named.into_iter().chain(fields).collect())
        }
        other => other,
    }
}

fn descriptor_in_oras_order(value: Value) -> Value {
    ordered(value, &DESCRIPTOR_ORDER)
}

fn manifest_in_oras_order(manifest: &ImageManifest) -> Result<Value, String> {
    let value =
        serde_json::to_value(manifest).map_err(|e| format!("cannot encode the manifest: {e}"))?;
    let mut value = ordered(value, &MANIFEST_ORDER);
    for key in [CONFIG_KEY, SUBJECT_KEY] {
        if let Some(field) = value.get_mut(key) {
            *field = descriptor_in_oras_order(field.take());
        }
    }
    if let Some(Value::Array(layers)) = value.get_mut(LAYERS_KEY) {
        for layer in layers.iter_mut() {
            *layer = descriptor_in_oras_order(layer.take());
        }
    }
    Ok(value)
}

fn validated_path(file: &Path, disable_path_validation: bool) -> Result<(), String> {
    if file.is_absolute() && !disable_path_validation {
        return Err(format!(
            "absolute file path detected. If it's intentional, use {DISABLE_PATH_VALIDATION_FLAG} flag to skip this check: {}",
            file.display()
        ));
    }
    Ok(())
}

fn uploaded(out: &mut Vec<u8>, digest: &str, what: &str) {
    out.extend_from_slice(format!("{UPLOADED_LABEL} {} {what}\n", short(digest)).as_bytes());
}

async fn push(
    client: &Client,
    auth: &RegistryAuth,
    name: &str,
    files: &[(PathBuf, String)],
    user_annotations: Vec<(String, String)>,
    settings: &PushOptions,
) -> Result<Vec<u8>, Failure> {
    let PushOptions {
        artifact_type,
        format,
        disable_path_validation,
    } = settings;
    let anonymous = matches!(auth, RegistryAuth::Anonymous);
    let image = reference(name)?;
    for (file, _) in files {
        validated_path(file, *disable_path_validation)?;
    }
    client
        .auth(&image, auth, RegistryOperation::Push)
        .await
        .map_err(|e| registry_failure(&format!("authenticate to {name}"), name, anonymous, e))?;
    let mut out = Vec::new();
    let mut layers = Vec::new();
    for (file, media_type) in files {
        let data = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let title = file_name(file)?;
        let layer = ImageLayer::new(data, media_type.clone(), None);
        let digest = layer.sha256_digest();
        client
            .push_blob(&image, layer.data.clone(), &digest)
            .await
            .map_err(|e| registry_failure(&format!("push {title}"), name, anonymous, e))?;
        uploaded(&mut out, &digest, &title);
        layers.push(descriptor(
            media_type,
            &digest,
            layer.data.len(),
            Some(annotations([(ANNOTATION_TITLE.to_string(), title)])?),
            None,
        )?);
    }
    let config = Config::new(EMPTY_CONFIG.as_bytes().to_vec(), empty_media_type(), None);
    let config_digest = config.sha256_digest();
    client
        .push_blob(&image, config.data.clone(), &config_digest)
        .await
        .map_err(|e| registry_failure("push the config", name, anonymous, e))?;
    uploaded(&mut out, &config_digest, &empty_media_type());
    let mut manifest_annotations: BTreeMap<String, String> = user_annotations.into_iter().collect();
    manifest_annotations
        .entry(ANNOTATION_CREATED.to_string())
        .or_insert(created_now()?);
    let manifest = manifest(artifact_type, layers, manifest_annotations.clone())?;
    let body = serde_json::to_vec(&manifest_in_oras_order(&manifest)?)
        .map_err(|e| format!("cannot encode the manifest: {e}"))?;
    let digest = sha256_digest(&body);
    client
        .push_manifest_raw(
            &image,
            body.clone(),
            HeaderValue::from_str(&manifest_media_type())
                .map_err(|e| format!("the manifest media type is not a header value: {e}"))?,
        )
        .await
        .map_err(|e| registry_failure("push the manifest", name, anonymous, e))?;
    uploaded(&mut out, &digest, &manifest_media_type());
    match format {
        Format::Text => {
            out.extend_from_slice(
                format!(
                    "{PUSHED_LABEL} {PUSHED_TARGET} {name}\n{ARTIFACT_TYPE_LABEL} {artifact_type}\n{DIGEST_LABEL} {digest}\n"
                )
                .as_bytes(),
            );
            Ok(out)
        }
        Format::Json => {
            let report = json!({
                "reference": format!("{}{DIGEST_SEPARATOR}{digest}", repository(&image)),
                "mediaType": manifest_media_type(),
                "digest": digest,
                "size": body.len(),
                "annotations": manifest_annotations,
                "artifactType": artifact_type,
                "referenceAsTags": [name],
            });
            let mut text = serde_json::to_vec_pretty(&report)
                .map_err(|e| format!("cannot encode the report: {e}"))?;
            text.push(b'\n');
            Ok(text)
        }
    }
}

async fn resolve(
    client: &Client,
    auth: &RegistryAuth,
    name: &str,
    full_reference: bool,
) -> Result<Vec<u8>, Failure> {
    let image = reference(name)?;
    let digest = client
        .fetch_manifest_digest(&image, auth)
        .await
        .map_err(|e| {
            registry_failure(
                "resolve digest",
                name,
                matches!(auth, RegistryAuth::Anonymous),
                e,
            )
        })?;
    let line = if full_reference {
        format!("{}{DIGEST_SEPARATOR}{digest}\n", repository(&image))
    } else {
        format!("{digest}\n")
    };
    Ok(line.into_bytes())
}

async fn manifest_fetch(
    client: &Client,
    auth: &RegistryAuth,
    name: &str,
    descriptor: bool,
    pretty: bool,
) -> Result<Vec<u8>, Failure> {
    let image = reference(name)?;
    let (bytes, _) = client
        .pull_manifest_raw(
            &image,
            auth,
            &[manifest_media_type().as_str(), IMAGE_MANIFEST_MEDIA_TYPE],
        )
        .await
        .map_err(|e| {
            registry_failure(
                &format!("fetch the content of \"{name}\""),
                name,
                matches!(auth, RegistryAuth::Anonymous),
                e,
            )
        })?;
    let manifest = bytes.to_vec();
    let value: Value = if descriptor {
        descriptor_in_oras_order(
            serde_json::to_value(descriptor_of(&manifest)?)
                .map_err(|e| format!("cannot encode the descriptor: {e}"))?,
        )
    } else if pretty {
        serde_json::from_slice(&manifest).map_err(|e| format!("the manifest is not JSON: {e}"))?
    } else {
        return Ok(manifest);
    };
    let encoded = if pretty {
        serde_json::to_vec_pretty(&value)
    } else {
        serde_json::to_vec(&value)
    };
    Ok(encoded.map_err(|e| format!("cannot encode the output: {e}"))?)
}

fn version() -> Vec<u8> {
    format!(
        "{VERSION_LABEL:<LABEL_WIDTH$}{VERSION}\n{OS_ARCH_LABEL:<LABEL_WIDTH$}{}/{}\n",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
    .into_bytes()
}

fn serve(request: Request) -> Result<Vec<u8>, Failure> {
    let Request { options, command } = request;
    if command == Command::Version {
        return Ok(version());
    }
    let auth = auth(&options)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the runtime: {e}"))?;
    let client = Client::new(ClientConfig {
        protocol: if options.plain_http {
            ClientProtocol::Http
        } else {
            ClientProtocol::Https
        },
        ..ClientConfig::default()
    });
    runtime.block_on(async {
        match command {
            Command::Push {
                name,
                files,
                annotations,
                settings,
            } => push(&client, &auth, &name, &files, annotations, &settings).await,
            Command::Resolve {
                name,
                full_reference,
            } => resolve(&client, &auth, &name, full_reference).await,
            Command::ManifestFetch {
                name,
                descriptor,
                pretty,
            } => manifest_fetch(&client, &auth, &name, descriptor, pretty).await,
            Command::Version => Ok(version()),
        }
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args).map_err(Failure::Local).and_then(serve) {
        Ok(bytes) => {
            if let Err(e) = std::io::stdout().write_all(&bytes) {
                eprintln!("{ERROR_PREFIX} cannot write to stdout: {e}");
                exit(1);
            }
        }
        Err(failure) => {
            eprintln!("{failure}");
            exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn push_takes_the_name_the_files_their_media_types_and_the_flags() {
        let request = parse(&words(
            "push -u bot --password-stdin -a a=1 --annotation b=x=y --format json \
             ghcr.io/zingolabs/zingolib/binding-layer-android:abc bundle.aar:application/zip plain",
        ))
        .unwrap();
        assert_eq!(
            request,
            Request {
                options: Options {
                    username: Some("bot".to_string()),
                    password: None,
                    password_from_stdin: true,
                    plain_http: false,
                },
                command: Command::Push {
                    name: "ghcr.io/zingolabs/zingolib/binding-layer-android:abc".to_string(),
                    files: vec![
                        (PathBuf::from("bundle.aar"), "application/zip".to_string()),
                        (PathBuf::from("plain"), layer_media_type()),
                    ],
                    annotations: vec![
                        ("a".to_string(), "1".to_string()),
                        ("b".to_string(), "x=y".to_string()),
                    ],
                    settings: PushOptions {
                        artifact_type: DEFAULT_ARTIFACT_TYPE.to_string(),
                        format: Format::Json,
                        disable_path_validation: false,
                    },
                },
            }
        );
    }

    #[test]
    fn an_absolute_file_path_is_refused_unless_validation_is_disabled() {
        assert!(validated_path(Path::new("/tmp/bundle.aar"), false).is_err());
        assert!(validated_path(Path::new("/tmp/bundle.aar"), true).is_ok());
        assert!(validated_path(Path::new("bundle.aar"), false).is_ok());
        assert!(parse(&words("resolve --disable-path-validation ghcr.io/x/y:z")).is_err());
    }

    #[test]
    fn the_manifest_keys_follow_the_order_oras_writes() {
        let layer = descriptor(
            "application/zip",
            "sha256:d5bd64a5c43b6f8cc7f6c7897e954e58b89abfed62e851a47f842de714cca7d0",
            16,
            Some(annotations([(ANNOTATION_TITLE.to_string(), "x.aar".to_string())]).unwrap()),
            None,
        )
        .unwrap();
        let manifest = manifest(DEFAULT_ARTIFACT_TYPE, vec![layer], BTreeMap::new()).unwrap();
        let text = serde_json::to_string(&manifest_in_oras_order(&manifest).unwrap()).unwrap();
        assert!(
            text.starts_with(r#"{"schemaVersion":2,"mediaType":"#),
            "{text}"
        );
        assert!(
            text.contains(r#""config":{"mediaType":"application/vnd.oci.empty.v1+json","digest":"sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a","size":2,"data":"e30="}"#),
            "{text}"
        );
        assert!(
            text.contains(r#""layers":[{"mediaType":"application/zip","digest":"sha256:d5bd64a5c43b6f8cc7f6c7897e954e58b89abfed62e851a47f842de714cca7d0","size":16,"annotations":{"org.opencontainers.image.title":"x.aar"}}]"#),
            "{text}"
        );
    }

    #[test]
    fn registry_failures_read_as_oras_reports_them() {
        let not_found = OciDistributionError::ImageManifestNotFoundError("x".to_string());
        assert_eq!(
            registry_failure("resolve digest", "r.io/a:b", true, not_found).to_string(),
            "Error response from registry: failed to resolve digest: r.io/a:b: not found"
        );
        let anonymous = OciDistributionError::UnauthorizedError {
            url: "http://r.io/v2/a/manifests/b".to_string(),
        };
        assert_eq!(
            registry_failure("resolve digest", "r.io/a:b", true, anonymous).to_string(),
            "Error: basic credential not found"
        );
        let refused = OciDistributionError::ServerError {
            url: "http://r.io/v2/a/blobs/uploads/".to_string(),
            code: 401,
            message: "{}".to_string(),
        };
        assert_eq!(
            registry_failure("push x", "r.io/a:b", false, refused).to_string(),
            "Error response from registry: \"http://r.io/v2/a/blobs/uploads/\": response status code 401: Unauthorized"
        );
    }

    #[test]
    fn a_file_with_a_colon_and_an_empty_type_keeps_its_name() {
        assert_eq!(
            file_and_media_type("hi:txt:"),
            (PathBuf::from("hi:txt"), layer_media_type())
        );
    }

    #[test]
    fn resolve_and_manifest_fetch_take_one_name_and_their_own_flags() {
        assert_eq!(
            parse(&words("resolve -l ghcr.io/x/y:z")).unwrap().command,
            Command::Resolve {
                name: "ghcr.io/x/y:z".to_string(),
                full_reference: true,
            }
        );
        assert_eq!(
            parse(&words("manifest fetch --descriptor --pretty ghcr.io/x/y:z"))
                .unwrap()
                .command,
            Command::ManifestFetch {
                name: "ghcr.io/x/y:z".to_string(),
                descriptor: true,
                pretty: true,
            }
        );
        assert_eq!(parse(&words("version")).unwrap().command, Command::Version);
    }

    #[test]
    fn a_flag_of_another_command_a_bare_key_a_bad_format_and_an_unknown_flag_are_refused() {
        assert!(parse(&words("resolve --annotation a=1 ghcr.io/x/y:z")).is_err());
        assert!(parse(&words("push --descriptor ghcr.io/x/y:z f")).is_err());
        assert!(parse(&words("push --annotation =1 ghcr.io/x/y:z f")).is_err());
        assert!(parse(&words("push --format yaml ghcr.io/x/y:z f")).is_err());
        assert!(parse(&words("push --quiet ghcr.io/x/y:z f")).is_err());
        assert!(parse(&words("push ghcr.io/x/y:z")).is_err());
        assert!(parse(&words("manifest ghcr.io/x/y:z")).is_err());
    }

    #[test]
    fn a_username_needs_a_password_and_a_password_needs_a_username() {
        let lone_user = Options {
            username: Some("bot".to_string()),
            ..Options::default()
        };
        assert!(auth(&lone_user).is_err());
        let lone_password = Options {
            password: Some("secret".to_string()),
            ..Options::default()
        };
        assert!(auth(&lone_password).is_err());
        assert!(matches!(
            auth(&Options::default()),
            Ok(RegistryAuth::Anonymous)
        ));
        let both = Options {
            username: Some("bot".to_string()),
            password: Some("secret".to_string()),
            ..Options::default()
        };
        assert!(matches!(auth(&both), Ok(RegistryAuth::Basic(_, _))));
    }

    #[test]
    fn the_created_annotation_is_rfc3339_utc() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_760_000_000), "2025-10-09T08:53:20Z");
    }

    #[test]
    fn the_manifest_is_shaped_as_oras_shapes_it() {
        let layer = descriptor(
            "application/zip",
            "sha256:d5bd64a5c43b6f8cc7f6c7897e954e58b89abfed62e851a47f842de714cca7d0",
            16,
            Some(
                annotations([(
                    ANNOTATION_TITLE.to_string(),
                    "zingo-android.aar".to_string(),
                )])
                .unwrap(),
            ),
            None,
        )
        .unwrap();
        let manifest = serde_json::to_value(
            manifest(
                DEFAULT_ARTIFACT_TYPE,
                vec![layer],
                [(
                    ANNOTATION_CREATED.to_string(),
                    "2026-10-09T20:51:30Z".to_string(),
                )]
                .into(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["schemaVersion"], 2);
        assert_eq!(manifest["mediaType"], manifest_media_type());
        assert_eq!(manifest["artifactType"], DEFAULT_ARTIFACT_TYPE);
        assert_eq!(manifest["config"]["mediaType"], empty_media_type());
        assert_eq!(
            manifest["config"]["digest"],
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
        assert_eq!(manifest["config"]["size"], 2);
        assert_eq!(manifest["config"]["data"], EMPTY_CONFIG_DATA);
        assert_eq!(
            manifest["layers"][0]["annotations"][ANNOTATION_TITLE],
            "zingo-android.aar"
        );
        assert_eq!(
            manifest["annotations"][ANNOTATION_CREATED],
            "2026-10-09T20:51:30Z"
        );
    }

    #[test]
    fn the_descriptor_carries_the_media_type_the_digest_and_the_size_as_oras_does() {
        let manifest = br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","artifactType":"application/vnd.unknown.artifact.v1","config":{"mediaType":"application/vnd.oci.empty.v1+json","digest":"sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a","size":2},"layers":[],"annotations":{"k":"v"}}"#;
        let descriptor = descriptor_in_oras_order(
            serde_json::to_value(descriptor_of(manifest).unwrap()).unwrap(),
        );
        assert_eq!(
            serde_json::to_string(&descriptor).unwrap(),
            format!(
                r#"{{"mediaType":"{}","digest":"{}","size":{}}}"#,
                manifest_media_type(),
                sha256_digest(manifest),
                manifest.len()
            )
        );
    }

    #[test]
    fn a_short_digest_is_twelve_hex_characters() {
        assert_eq!(
            short("sha256:c0215f32d3ae0ca771ad9f1f6549d05d"),
            "c0215f32d3ae"
        );
    }
}
