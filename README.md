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
Errors print as `Error: <message>` or `Error response from registry:
<message>` on stderr with exit status 1, in the words oras uses.

Parity was measured against oras 1.3.4 on a local registry with and without
basic authentication. Pushing the same file with the same annotations through
both tools yields byte-identical manifests apart from the creation second,
and the descriptor and the not-found error lines are identical.

## The specifications in the code

The manifest and descriptor types are generated at build time from the OCI
image spec's JSON Schema, vendored under `schema/` at the commit `SOURCE`
names. The build script bundles the schema's cross-file references into one
document and hands it to [typify](https://github.com/oxidecomputer/typify),
so a digest, a media type or an annotation key that the spec's patterns
reject is refused before anything reaches the registry.

The spec's named values that no schema carries come from
[oci-spec](https://github.com/containers/oci-spec-rs): the media types of a
manifest, a layer and the empty config, and the annotation keys for a title
and a creation time. A reference is parsed against the distribution spec's
grammar from the same crate before the client sees it. The distribution
protocol itself, with its token exchange, blob upload sessions and manifest
requests, is oci-client's.

The oras stack carries no other interface definition. oras is a cobra
program whose contract is its help text, oras-go is a library, and the
distribution spec is prose with a Go conformance suite, so an OpenAPI
generator has nothing to consume there.

## Installing

The crate is published on crates.io, so a consumer installs the version it
pins and checks it against `orasust version`:

```
cargo install --locked orasust@0.1.3
```

A tag `v<version>` also builds the Linux x86_64 binary and attaches it to a
GitHub release as `orasust-x86_64-unknown-linux-gnu`. The tag must equal the
crate version.
