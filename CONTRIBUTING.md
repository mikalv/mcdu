# Contributing to mcdu

Thanks for your interest in contributing! This document provides guidelines and instructions for contributing to mcdu.

## Code of Conduct

Please be respectful and constructive in all interactions.

## Getting Started

1. **Fork the repository** on GitHub
2. **Clone your fork** locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/mcdu.git
   cd mcdu
   ```
3. **Create a branch** for your changes:
   ```bash
   git checkout -b feature/your-feature-name
   ```

## Development Setup

### Prerequisites
- Rust 1.70+ (install from https://rustup.rs/)
- macOS or Linux

### Building
```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run with debug logging
RUST_LOG=debug cargo run
```

### Testing
```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run a specific test
cargo test test_name
```

### Code Quality

We enforce code quality through several tools:

```bash
# Format code
cargo fmt

# Check formatting
cargo fmt -- --check

# Lint with Clippy
cargo clippy -- -D warnings

# Run all checks
cargo check
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## Contribution Guidelines

### Before Starting
- Check existing issues to avoid duplicate work
- For large features, open a discussion issue first
- Ask questions if anything is unclear

### Making Changes

1. **Keep commits atomic** - One logical change per commit
2. **Write clear commit messages**:
   ```
   Brief summary (50 chars)

   More detailed explanation of what changed and why
   ```
3. **Update documentation** - Keep README and code comments current
4. **Add tests** for new functionality
5. **Run full test suite** before submitting

### Code Style

- Follow Rust conventions (enforced by `rustfmt`)
- Use meaningful variable names
- Add comments for non-obvious logic
- Keep functions focused and small

### Performance Considerations

- Be mindful of large directory operations
- Consider using iterators rather than collecting into vectors when possible
- Profile before optimizing

## Submitting Changes

1. **Push to your fork**:
   ```bash
   git push origin feature/your-feature-name
   ```

2. **Open a Pull Request** on GitHub with:
   - Clear title and description
   - Reference to related issues
   - Screenshots for UI changes
   - Explanation of testing

3. **Respond to feedback** - We may request changes before merging

## PR Requirements

For PRs to be merged:
- ✅ All checks pass (CI/CD pipeline)
- ✅ Code is formatted (`cargo fmt`)
- ✅ No clippy warnings (`cargo clippy -- -D warnings`)
- ✅ Tests pass (`cargo test`)
- ✅ At least one maintainer review

## Areas for Contribution

### Good First Issues
- Documentation improvements
- Error message enhancements
- Test coverage
- UI polish

### Wanted Features
- [ ] Windows support
- [ ] Configuration file support
- [ ] Undo functionality
- [ ] Search/filter capabilities
- [ ] Performance improvements
- [ ] Parallel deletion with rayon

## Architecture Overview

### Key Modules

- **main.rs** - Event loop and input handling
- **app.rs** - Application state and core logic
- **ui.rs** - Terminal UI rendering
- **scan.rs** - Directory scanning
- **delete.rs** - File deletion
- **modal.rs** - Dialog system
- **changes.rs** - Change detection and fingerprinting
- **logger.rs** - Deletion audit logging

### Data Flow

1. User input → main.rs → handle_input()
2. Input → app.rs state changes
3. State → ui.rs for rendering
4. Deletion → background thread → logger.rs

## Testing

### Unit Tests

Add tests in the same file as the code being tested:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_something() {
        // Test code
    }
}
```

### Integration Tests

Create files in `tests/` directory for integration tests.

## Documentation

- Add doc comments to public APIs: `/// Documentation`
- Keep README up to date with features
- Update FEATURES.md for new capabilities
- Add comments for complex algorithms

## Performance & Optimization

- Profile before optimizing
- Avoid blocking the UI event loop
- Use quick estimates for large directory sizes
- Cache computed values when appropriate

## Platform-Specific Code

Platform-specific code goes in `src/platform.rs`:

```rust
#[cfg(target_os = "macos")]
fn platform_specific() {
    // macOS code
}
```

## Getting Help

- Open an issue with the `question` label
- Check existing issues for similar questions
- Ask in PRs if you need clarification

## License

By contributing, you agree that your contributions will be licensed under the MIT License.

---

**Questions?** Open an issue or reach out to the maintainers!
