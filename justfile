set shell := ["bash", "-cu"]
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# Run SQL Server 2022 via Docker (accept the EULA, set a strong SA password).
run-sqlserver:
    docker run -d --name sqlserver-dev -p 1433:1433 \
        -e "ACCEPT_EULA=Y" -e "MSSQL_SA_PASSWORD=Str0ng!Passw0rd" \
        mcr.microsoft.com/mssql/server:2022-latest

# Stop and remove the local SQL Server container.
stop-sqlserver:
    docker rm -f sqlserver-dev

# Seed a test database into the local SQL Server container. CREATE DATABASE
# must finish in its own batch before sqlcmd selects the new database.
seed-sqlserver:
    docker exec sqlserver-dev /opt/mssql-tools18/bin/sqlcmd \
        -S localhost -U sa -P "Str0ng!Passw0rd" -C -Q \
        "IF DB_ID('tabularis_test') IS NULL CREATE DATABASE tabularis_test;"
    docker exec sqlserver-dev /opt/mssql-tools18/bin/sqlcmd \
        -S localhost -U sa -P "Str0ng!Passw0rd" -C -d tabularis_test -Q \
        "IF OBJECT_ID('dbo.users') IS NULL BEGIN \
           CREATE TABLE dbo.users (id INT IDENTITY(1,1) PRIMARY KEY, name NVARCHAR(100) NOT NULL, email NVARCHAR(255) NOT NULL); \
           INSERT INTO dbo.users (name, email) VALUES (N'Alice', N'alice@example.com'), (N'Bob', N'bob@example.com'); \
         END"

# ---------------------------------------------------------------------------
# Cross-platform recipes (only shell-agnostic tooling — cargo, npm, pnpm).
# ---------------------------------------------------------------------------

# Build the plugin binary and its optional JavaScript artifacts.
build: build-ui build-explain
    cargo build

# Build for release (what the GitHub Actions workflow ships).
release: build-ui build-explain
    cargo build --release

# Build and test the browser-safe SQL Server SHOWPLAN parser package.
build-explain:
    pnpm --dir explain install --frozen-lockfile
    pnpm --dir explain build

test-explain:
    pnpm --dir explain install --frozen-lockfile
    pnpm --dir explain typecheck
    pnpm --dir explain test

# Run unit tests only. This crate is binary-only, so --lib would fail; --bins
# also keeps tests/live_db.rs out of the default run.
test:
    cargo test --bins

# Launch the local REPL that simulates Tabularis JSON-RPC calls over stdio.
repl:
    cargo run --bin test_plugin

# Run clippy on the workspace.
lint:
    cargo clippy --all-targets -- -D warnings

# Format the codebase.
fmt:
    cargo fmt --all

# ---------------------------------------------------------------------------
# Platform-specific recipes (file operations + plugin-dir conventions).
#
# Host source: tabularis/src-tauri/src/plugins/manager.rs loads the directory
# returned by tabularis/src-tauri/src/plugins/installer.rs::get_plugins_dir,
# which appends `plugins` to tabularis/src-tauri/src/paths.rs::get_app_data_dir.
# paths.rs uses ProjectDirs("", "", "tabularis") and removes the directories
# crate's Windows `data` leaf. The resulting roots are
# ${XDG_DATA_HOME:-$HOME/.local/share}/tabularis/plugins on Linux,
# $HOME/Library/Application Support/tabularis/plugins on macOS, and
# %APPDATA%\tabularis\plugins on Windows.
# ---------------------------------------------------------------------------

# Build the UI extension if present (no-op otherwise).
[unix]
build-ui:
    @if [ -f ui/package.json ]; then \
        echo "Building UI extension..."; \
        (cd ui && npm install --no-audit --no-fund && npm run build); \
    fi

[windows]
build-ui:
    if (Test-Path "ui\package.json") { \
        Write-Host "Building UI extension..."; \
        Push-Location ui; \
        try { \
            npm install --no-audit --no-fund; \
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; \
            npm run build; \
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; \
        } finally { \
            Pop-Location; \
        }; \
    }

# Build + copy binary, manifest and optional bundles into Tabularis's plugin folder.
[linux]
dev-install: build
    mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/tabularis/plugins/sqlserver"
    cp target/debug/sqlserver-plugin "${XDG_DATA_HOME:-$HOME/.local/share}/tabularis/plugins/sqlserver/"
    cp .tabularium "${XDG_DATA_HOME:-$HOME/.local/share}/tabularis/plugins/sqlserver/"
    @if [ -f ui/dist/index.js ]; then \
        mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/tabularis/plugins/sqlserver/ui/dist"; \
        cp ui/dist/index.js "${XDG_DATA_HOME:-$HOME/.local/share}/tabularis/plugins/sqlserver/ui/dist/"; \
    fi
    @if [ -f explain/dist/index.iife.js ]; then \
        mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/tabularis/plugins/sqlserver/explain/dist"; \
        cp explain/dist/index.iife.js "${XDG_DATA_HOME:-$HOME/.local/share}/tabularis/plugins/sqlserver/explain/dist/"; \
    fi
    @echo "Installed to ${XDG_DATA_HOME:-$HOME/.local/share}/tabularis/plugins/sqlserver"
    @echo "Restart Tabularis (or toggle the plugin in Settings) to pick up changes."

[macos]
dev-install: build
    mkdir -p "$HOME/Library/Application Support/tabularis/plugins/sqlserver"
    cp target/debug/sqlserver-plugin "$HOME/Library/Application Support/tabularis/plugins/sqlserver/"
    cp .tabularium "$HOME/Library/Application Support/tabularis/plugins/sqlserver/"
    @if [ -f ui/dist/index.js ]; then \
        mkdir -p "$HOME/Library/Application Support/tabularis/plugins/sqlserver/ui/dist"; \
        cp ui/dist/index.js "$HOME/Library/Application Support/tabularis/plugins/sqlserver/ui/dist/"; \
    fi
    @if [ -f explain/dist/index.iife.js ]; then \
        mkdir -p "$HOME/Library/Application Support/tabularis/plugins/sqlserver/explain/dist"; \
        cp explain/dist/index.iife.js "$HOME/Library/Application Support/tabularis/plugins/sqlserver/explain/dist/"; \
    fi
    @echo "Installed to ~/Library/Application Support/tabularis/plugins/sqlserver"
    @echo "Restart Tabularis (or toggle the plugin in Settings) to pick up changes."

# Each recipe line runs in a fresh shell, so this must be one logical command.
[windows]
dev-install: build
    $dest = Join-Path $env:APPDATA "tabularis\plugins\sqlserver"; \
    New-Item -ItemType Directory -Force -Path $dest | Out-Null; \
    Copy-Item "target\debug\sqlserver-plugin.exe" $dest; \
    Copy-Item ".tabularium" $dest; \
    if (Test-Path "ui\dist\index.js") { \
        New-Item -ItemType Directory -Force -Path (Join-Path $dest "ui\dist") | Out-Null; \
        Copy-Item "ui\dist\index.js" (Join-Path $dest "ui\dist"); \
    }; \
    if (Test-Path "explain\dist\index.iife.js") { \
        New-Item -ItemType Directory -Force -Path (Join-Path $dest "explain\dist") | Out-Null; \
        Copy-Item "explain\dist\index.iife.js" (Join-Path $dest "explain\dist"); \
    }; \
    Write-Host "Installed to $dest"; \
    Write-Host "Restart Tabularis (or toggle the plugin in Settings) to pick up changes."

# Remove the installed plugin from the same host-defined directory.
[linux]
uninstall:
    rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/tabularis/plugins/sqlserver"

[macos]
uninstall:
    rm -rf "$HOME/Library/Application Support/tabularis/plugins/sqlserver"

[windows]
uninstall:
    $dest = Join-Path $env:APPDATA "tabularis\plugins\sqlserver"; \
    if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
