Name:           mcdu
Version:        0.6.0
Release:        1%{?dist}
Summary:        Modern disk usage analyzer with TUI and developer cleanup tools

License:        MIT
URL:            https://github.com/mikalv/mcdu
Source0:        https://github.com/mikalv/%{name}/archive/refs/tags/v%{version}.tar.gz

BuildRequires:  rust >= 1.70
BuildRequires:  cargo
BuildRequires:  gcc

%description
mcdu is a fast, modern disk usage analyzer with an integrated developer
cleanup tool. Think ncdu meets CCleaner for developers.

Features:
- Disk usage browser with vim-style navigation
- Developer cleanup scanning 18+ ecosystems
- Safe deletion with confirmation and audit logging
- Background async scanning with live progress
- Cross-platform: macOS and Linux

%prep
%autosetup -n %{name}-%{version}

%build
cargo build --release --locked

%check
cargo test --release --locked

%install
install -Dm755 target/release/%{name} %{buildroot}%{_bindir}/%{name}

%files
%license LICENSE
%{_bindir}/%{name}

%changelog
* Wed Feb 12 2026 Mikal Villa <spam@mux.rs> - 0.5.0-1
- Developer cleanup mode with 18+ ecosystem support
- Quarantine system, background scanning, custom rules

* Wed Jan 01 2025 Mikal Villa <spam@mux.rs> - 0.3.5-1
- Initial package
