# GitHub Actions Workflows

This directory contains GitHub Actions workflows for CI/CD automation.

## Workflows

### 1. CI (ci.yml)

**Triggers:** Push to main, Pull Requests

**Jobs:**
- **Format Check**: Validates code formatting with `rustfmt`
- **Clippy Lint**: Runs static analysis with all warnings as errors
- **Test Suite**: Runs tests on Linux, macOS, and Windows
- **Build Check**: Validates debug and release builds

**Purpose:** Ensures code quality on every commit and PR.

### 2. Build (build.yml)

**Triggers:** Push to main, Pull Requests, Manual dispatch

**Platforms:**
- Linux x86_64 (glibc)
- Linux x86_64 (musl - static binary)
- macOS x86_64 (Intel)
- macOS ARM64 (Apple Silicon)
- Windows x86_64

**Artifacts:**
- Compiled binaries for all platforms
- SHA256 checksums for verification

**Purpose:** Creates distributable binaries for testing.

### 3. Release (release.yml)

**Triggers:** Git tags matching `v*.*.*` (e.g., v1.0.0), Manual dispatch

**Process:**
1. Run all tests and quality checks
2. Build binaries for all platforms
3. Create compressed archives (.tar.gz for Unix, .zip for Windows)
4. Generate SHA256 checksums
5. Create GitHub Release with auto-generated notes
6. Upload all binaries and checksums to release

**Purpose:** Fully automated release process.

## Usage

### Running CI locally

Before pushing, you can run the same checks locally:

```bash
# Format check
cargo fmt --check

# Clippy lint
cargo clippy --all-targets --all-features -- -D warnings

# Tests
cargo test --all --verbose

# Build
cargo build --release
```

### Creating a release

1. **Update version in Cargo.toml** (if needed)
2. **Commit changes**
3. **Create and push a tag:**
   ```bash
   git tag v1.0.0
   git push origin v1.0.0
   ```
4. **GitHub Actions will automatically:**
   - Run tests
   - Build binaries for all platforms
   - Create a GitHub Release
   - Upload binaries and checksums

### Manual release trigger

You can also trigger a release manually:

1. Go to Actions → Release workflow
2. Click "Run workflow"
3. Enter the tag name (e.g., `v1.0.0`)
4. Click "Run workflow"

## Requirements

All workflows use:
- **Rust toolchain:** Latest stable via dtolnay/rust-toolchain
- **Caching:** Cargo registry, git index, and target directories
- **Cross-compilation:** Native tooling for each platform

## Notes

- **CI runs on every PR** - Ensure your code passes before merging
- **Build artifacts expire** - Download them within GitHub's retention period
- **Release binaries are permanent** - Uploaded to GitHub Releases
- **Checksums are provided** - Always verify downloaded binaries

## Troubleshooting

### CI failures

- Check the Actions tab for detailed logs
- Run the same commands locally to reproduce
- Ensure all dependencies are in Cargo.lock

### Build failures

- Verify target support: `rustup target list`
- Check for platform-specific dependencies
- Review error logs in the Actions tab

### Release failures

- Ensure the tag follows semver (v1.2.3)
- Check repository permissions for GITHUB_TOKEN
- Verify all tests pass before tagging
