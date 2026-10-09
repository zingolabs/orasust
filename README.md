# orasust

The minimal subset of the [oras](https://oras.land) command line that the
zingolib Binding Layer publication needs, written in Rust over
[oci-client](https://github.com/oras-project/rust-oci-client), the oras
project's own Rust library.

The binary offers three commands and a version command. Each one spells its
arguments and its output the way oras 1.3 does, so a reader who knows oras
knows this tool, and a script written against oras runs against this binary.

```
orasust push [flags] <name>[:<tag>] <file>[:<type>] [...]
orasust resolve [flags] <name>{:<tag>|@<digest>}
orasust manifest fetch [flags] <name>{:<tag>|@<digest>}
orasust version
```

`push` uploads each file as one layer under the empty OCI config, annotates
each layer with its file name as the title, adds the creation time to the
manifest annotations beside every `--annotation`, and prints `Pushed`,
`ArtifactType:` and `Digest:` lines, or one JSON object with `--format json`.
`resolve` prints the digest a reference serves, with the repository in front
under `--full-reference`. `manifest fetch` prints the manifest as the registry
serves it, its descriptor under `--descriptor`, and either one prettified
under `--pretty`. Credentials come from `--username` with `--password` or
`--password-stdin`; without them every command asks the registry anonymously.
Errors print as `Error: <message>` on stderr with exit status 1.

## Manifest types from the image spec

The manifest and descriptor types are generated at build time from the OCI
image spec's JSON Schema, vendored under `schema/` at the commit `SOURCE`
names. The build script bundles the schema's cross-file references into one
document and hands it to [typify](https://github.com/oxidecomputer/typify),
so a digest, a media type or an annotation key that the spec's patterns
reject is refused before anything reaches the registry.

## Releases

A tag `v<version>` builds the Linux x86_64 binary and attaches it to a GitHub
release as `orasust-x86_64-unknown-linux-gnu`. The tag must equal the crate
version. A consumer pins the version it installs and checks it against
`orasust version`.
