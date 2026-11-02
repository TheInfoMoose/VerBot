Name:           verbot
Version:        0.1.0
Release:        1%{?dist}
Summary:        Verbocity (verbot) — Interactive terminal LLM prompt template engine and agy dispatcher
License:        MIT
URL:            https://github.com/TheInfoMoose/VerBot
Source0:        %{url}/archive/refs/tags/v%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
Requires:       bash

%description
Verbocity (verbot) is a multi-pane Ratatui terminal prompt builder, template engine,
and Antigravity (agy) CLI dispatcher styled in the NovOS Cosmic Nova Orange and
COSMIC DE palette.

%prep
%autosetup -n VerBot-%{version}

%build
cargo build --release --locked

%install
install -Dm0755 bin/verbot %{buildroot}%{_bindir}/verbot
install -Dm0755 target/release/verbot-tui %{buildroot}%{_bindir}/verbot-tui
install -Dm0755 scripts/dispatch-agy.sh %{buildroot}%{_datadir}/verbot/scripts/dispatch-agy.sh
install -Dm0755 scripts/dispatch-output.sh %{buildroot}%{_datadir}/verbot/scripts/dispatch-output.sh
install -Dm0644 assets/verbot-logo.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/verbot.svg
install -Dm0644 assets/verbot.desktop %{buildroot}%{_datadir}/applications/verbot.desktop
install -Dm0644 templates/GeminiAgentPromptTemplate.md %{buildroot}%{_datadir}/verbot/templates/GeminiAgentPromptTemplate.md
install -Dm0644 templates/GeminiAgentPromptTemplate.toml %{buildroot}%{_datadir}/verbot/templates/GeminiAgentPromptTemplate.toml
install -Dm0644 README.md %{buildroot}%{_docdir}/verbot/README.md

%files
%{_bindir}/verbot
%{_bindir}/verbot-tui
%{_datadir}/verbot/scripts/dispatch-agy.sh
%{_datadir}/verbot/scripts/dispatch-output.sh
%{_datadir}/icons/hicolor/scalable/apps/verbot.svg
%{_datadir}/applications/verbot.desktop
%{_datadir}/verbot/templates/GeminiAgentPromptTemplate.md
%{_datadir}/verbot/templates/GeminiAgentPromptTemplate.toml
%{_docdir}/verbot/README.md

%changelog
* Sun Nov 01 2026 Verbocity Maintainers <maintainers@novos.dev> - 0.1.0-1
- Initial release of Verbocity (verbot)
