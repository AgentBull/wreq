# wreq

> **AgentBull Internal Patched Clone (`AgentBull/wreq`)**
>
> This repository is an internal patched clone of [`0x676e67/wreq`](https://github.com/0x676e67/wreq), maintained as a **single self-contained commit** on top of upstream `main`. It is referenced by `relay_native` in `AgentBull/saas` via Cargo's `[patch.crates-io]` with a pinned `rev`.
>
> ### Why We Maintain This Patch
>
> AgentBull Relay's outbound transport (`relay_native::transport`) has two requirements that upstream `wreq` does not support out of the box:
>
> 1. **Proxy Tunnel Target Pinning (`wreq::conn::TunnelTarget`) — SSRF & DNS Rebinding Defense**
>    - **Problem**: Before opening an outbound connection, Relay resolves the upstream hostname locally and validates every resolved IP against private-network, loopback, link-local, and cloud-metadata blocklists. For direct connections, a custom DNS resolver pins the connection to those pre-validated IPs. However, when a request goes through an HTTP `CONNECT` or `SOCKS5` proxy, upstream `wreq` (`connect_via_proxy`) derives the tunnel target (`CONNECT host:port` authority or `SOCKS5` target address) directly from the request `Uri`, bypassing the custom DNS resolver. Rewriting the request `Uri` host to the validated IP is not an option because doing so corrupts the HTTP `Host` / `:authority` header, the TLS SNI server name, and X.509 certificate hostname verification.
>    - **Patch (P1)**: Adds a per-request extension `wreq::conn::TunnelTarget` and threads it through `ConnectionDescriptor` into `connect_via_proxy` and `socks::connect`. When `TunnelTarget` is present on a request, the HTTP `CONNECT` authority and `SOCKS5` target use the pre-validated IP address while the request `Uri`, `Host` / `:authority`, TLS SNI, and certificate verification continue to use the original upstream hostname.
>
> 2. **Wire-Order HTTP/1.1 Header Emission (`wreq::header::OrderedHeaders`) — Client Impersonation Fidelity**
>    - **Problem**: Official LLM CLI clients (such as Claude Code and Codex CLI) send headers in a specific order and casing. Standard `http::HeaderMap` lowercases all header names and does not guarantee iteration order across interleaved duplicate headers. Upstream `wreq` provides `OrigHeaderMap`, but `OrigHeaderMap` groups headers by lowercase name and retains only the first casing for duplicate header names, so interleaved duplicate headers (`X-Trace: 1`, `Accept: */*`, `x-trace: 2`) cannot be represented with their exact sequence and per-occurrence spelling.
>    - **Patch (P2)**: Adds a per-request extension `wreq::header::OrderedHeaders` carrying `Vec<(Bytes, HeaderValue)>`. In `ConfigService`, when `OrigHeaderMap` is not set, a non-empty `OrderedHeaders` extension installs an internal `netty::ext::OnPreserveHeaderCallback` implementation that writes every listed header in exact wire order and original name bytes on HTTP/1.1, then drains any remaining unlisted entries (such as computed `Content-Length`) from `HeaderMap` without exposing internal `netty` callback traits in `wreq`'s public API.
>
> ### Patch Footprint & Upstream Sync Workflow
>
> All changes are kept in **one single commit** on top of upstream `main`, touching only 8 files:
> - `src/lib.rs`
> - `src/conn.rs`
> - `src/conn/descriptor.rs`
> - `src/conn/connector.rs`
> - `src/conn/proxy/socks.rs`
> - `src/client/layer/client.rs`
> - `src/client/layer/config.rs`
> - `src/header.rs`
>
> To sync with upstream `0x676e67/wreq`:
> ```bash
> git fetch upstream
> git rebase --onto upstream/main HEAD^
> git push --force-with-lease origin main
> ```
> After pushing, update the `wreq` `rev` (and matching `btls` / `http2` / `netty` `[patch.crates-io]` revs from `wreq/Cargo.toml`) in `AgentBull/saas` (`control_plane/native/relay_native/Cargo.toml`) so `Cargo.lock` and CI caches update atomically.

[![CI](https://github.com/0x676e67/wreq/actions/workflows/ci.yml/badge.svg)](https://github.com/0x676e67/wreq/actions/workflows/ci.yml)
[![Crates.io License](https://img.shields.io/crates/l/wreq)](https://github.com/0x676e67/wreq/blob/main/LICENSE)
[![Crates.io MSRV](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fcrates.io%2Fapi%2Fv1%2Fcrates%2Fwreq%2Fversions%3Fsort%3Ddate%26per_page%3D1&query=%24.versions%5B0%5D.rust_version&suffix=.0&label=MSRV&color=blue&logo=rust)](https://crates.io/crates/wreq)
[![crates.io](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fcrates.io%2Fapi%2Fv1%2Fcrates%2Fwreq&query=%24.crate.max_stable_version&prefix=v&label=crates.io&color=orange&logo=rust)](https://crates.io/crates/wreq)
[![Discord chat][discord-badge]][discord-url]

[discord-badge]: https://img.shields.io/discord/1486741856397164788.svg?logo=discord
[discord-url]: https://discord.gg/rfbvyFkgq3

> 🚀 Help me work seamlessly with open source sharing by [sponsoring me on GitHub](https://github.com/0x676e67/0x676e67/blob/main/SPONSOR.md)

An ergonomic and modular Rust HTTP Client for high-fidelity protocol matching, featuring customizable TLS, JA3/JA4, and HTTP/2 signature capabilities.

## Features

- Plain bodies, JSON, urlencoded, multipart
- HTTP Trailer
- Cookie Store
- Redirect Policy
- Original Header
- Rotating Proxies
- Tower Middleware
- WebSocket Upgrade
- HTTPS via BoringSSL
- HTTP/2 over TLS Parity
- Certificate Store (CAs & mTLS)
- Multiple Runtime (Tokio, Compio)

## Example

The following example uses the [Tokio](https://tokio.rs) runtime with optional features enabled by adding this to your `Cargo.toml`:

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
wreq = "0.16"
wreq-util = "0.2"
```

And then the code:

```rust
use wreq::Client;
use wreq_util::Emulation;

#[tokio::main]
async fn main() -> wreq::Result<()> {
    // Build a client
    let client = Client::builder()
        .emulation(Emulation::Safari26)
        .build()?;

    // Use the API you're already familiar with
    let resp = client.get("https://pingly.us.kg/api/all").send().await?;
    println!("{}", resp.text().await?);
    Ok(())
}
```

## Behavior

- **HTTP/1 over TLS**

In the Rust ecosystem, most HTTP clients rely on the [http](https://github.com/hyperium/http) library, which performs well but does not preserve header case. This causes some **WAFs** to reject **HTTP/1** requests with lowercase headers (see [discussion](https://github.com/seanmonstar/reqwest/discussions/2227)). **wreq** addresses this by fully supporting **HTTP/1** header case sensitivity.

- **HTTP/2 over TLS**

Due to the complexity of **TLS** encryption and the widespread adoption of **HTTP/2**, browser fingerprints such as **JA3**, **JA4**, and **Akamai** cannot be reliably emulated using simple fingerprint strings. Instead of parsing and emulating these string-based fingerprints, **wreq** provides fine-grained control over **TLS** and **HTTP/2** extensions and settings for precise browser behavior emulation.

- **Device Emulation**

**TLS** and **HTTP/2** fingerprints are often identical across various browser models because these underlying protocols evolve slower than browser release cycles. **100+ browser device emulation profiles** are maintained in [wreq-util](https://github.com/0x676e67/wreq-util).

## MSRV and Edition

The current MSRV is 1.98. The current edition is Rust 2024.

`wreq` has a rolling MSRV (Minimum Supported Rust Version) policy to support the current and previous Rust
compiler versions, and the latest [edition of Rust](https://doc.rust-lang.org/edition-guide/editions/index.html).

We may periodically lag the MSRV behind this latest version/edition in rare cases for our dependencies to catch up and
for us to perform any necessary
fixes.

## Building

Compiling alongside **openssl-sys** can cause symbol conflicts with **boringssl** that lead to [link failures](https://github.com/cloudflare/boring/issues/197), and on **Linux** and **Android** this can be avoided by enabling the **`prefix-symbols`** feature.

Install [BoringSSL build dependencies](https://github.com/google/boringssl/blob/master/BUILDING.md#build-prerequisites) and build with:

```bash
sudo apt-get install build-essential cmake perl pkg-config libclang-dev musl-tools git -y
cargo build --release
```

This GitHub Actions [workflow](.github/compilation-guide/build.yml) can be used to compile the project on **Linux**, **Windows**, and **macOS**.

## Services

Help sustain the ongoing development of this open-source project by reaching out for [commercial support](mailto:gngppz@gmail.com). Receive private guidance, expert reviews, or direct access to the maintainer, with personalized technical assistance tailored to your needs.

## License

Licensed under either of Apache License, Version 2.0 ([LICENSE](./LICENSE) or http://www.apache.org/licenses/LICENSE-2.0).

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the [Apache-2.0](./LICENSE) license, shall be licensed as above, without any additional terms or conditions.

## Sponsors

<a href="https://captcha.fun/?utm_source=github&utm_medium=readme&utm_campaign=wreq" target="_blank"><img src="https://www.captcha.fun/banner.jpg" height="47" width="149"></a>

**Solve reCAPTCHA in less than 2 seconds**

**[Captcha.fun](https://captcha.fun/?utm_source=github&utm_medium=readme&utm_campaign=wreq)** delivers fast, reliable CAPTCHA solving built for automation at scale.

With simple API integration, consistent performance, and competitive pricing, it's an easy way to keep your workflows moving without delays—use code **`WREQ`** for **10% bonus credits**.

**[Dashboard](https://dash.captcha.fun/)** | **[Docs](http://docs.captcha.fun/)** | **[Discord](https://discord.gg/captchafun)**

---

<a href="https://byteful.com/?utm_source=github_rust&utm_medium=github-sponsor&utm_campaign=wreq_github_sponsorship" target="_blank"><img src="./.github/assets/byteful-logo.svg" width="149" height="47" alt="Byteful"></a>

**35M+ residential IPs with 99.9% success rates**

UK-based **[Byteful](https://byteful.com/?utm_source=github_rust&utm_medium=github-sponsor&utm_campaign=wreq_github_sponsorship)** provides ethically sourced residential, mobile, static residential (ISP), and datacenter proxies. Its API-first scraping, data collection, and AI automation tools handle tens of billions of monthly requests.

With free geo-targeting and ~0.5s response times, Byteful helps you collect data at scale. Use code **`WREQ10`** for **10% off residential proxies**.

**[Dashboard](https://dashboard.byteful.com/)** | **[Docs](https://documentation.byteful.com/)** | **[Discord](https://discord.com/invite/ping-proxies-584021352940568578)**

---

<a href="https://hypersolutions.co/?utm_source=github&utm_medium=readme&utm_campaign=wreq" target="_blank"><img src="https://raw.githubusercontent.com/0x676e67/wreq/main/.github/assets/hypersolutions.jpg" height="47" width="149"></a>

TLS fingerprinting alone isn't enough for modern bot protection. **[Hyper Solutions](https://hypersolutions.co?utm_source=github&utm_medium=readme&utm_campaign=wreq)** provides the missing piece - API endpoints that generate valid antibot tokens for:

**Akamai** • **DataDome** • **Kasada** • **Incapsula**

No browser automation. Just simple API calls that return the exact cookies and headers these systems require.

**[Dashboard](https://hypersolutions.co?utm_source=github&utm_medium=readme&utm_campaign=wreq)** | **[Docs](https://docs.justhyped.dev)** | **[Discord](https://discord.gg/akamai)**

## Accolades

A hard fork of [reqwest](https://github.com/seanmonstar/reqwest).
