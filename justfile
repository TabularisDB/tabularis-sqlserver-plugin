set shell := ["bash", "-cu"]
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# Run SQL Server 2022 via Docker (accept the EULA, set a strong SA password)
run-sqlserver:
	docker run -d --name sqlserver-dev -p 1433:1433 \
		-e "ACCEPT_EULA=Y" -e "MSSQL_SA_PASSWORD=Str0ng!Passw0rd" \
		mcr.microsoft.com/mssql/server:2022-latest

# Seed a test database into the local SQL Server container
seed-sqlserver:
	docker exec sqlserver-dev /opt/mssql-tools18/bin/sqlcmd \
		-S localhost -U sa -P "Str0ng!Passw0rd" -C -Q \
		"IF DB_ID('tabularis_test') IS NULL CREATE DATABASE tabularis_test; \
		 USE tabularis_test; \
		 IF OBJECT_ID('dbo.users') IS NULL BEGIN \
		   CREATE TABLE dbo.users (id INT IDENTITY(1,1) PRIMARY KEY, name NVARCHAR(100) NOT NULL, email NVARCHAR(255) NOT NULL); \
		   INSERT INTO dbo.users (name, email) VALUES (N'Alice', N'alice@example.com'), (N'Bob', N'bob@example.com'); \
		 END"

# ---------------------------------------------------------------------------
# Cross-platform recipes (only shell-agnostic tooling — cargo, npm).
# ---------------------------------------------------------------------------

# Build the plugin binary in debug mode (plus UI if present).
build: build-ui
    cargo build

# Build for release (what the GitHub Actions workflow ships).
release: build-ui
    cargo build --release

# Run unit tests.
test:
    cargo test

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
    if (Test-Path ui/package.json) {
        Write-Host "Building UI extension..."
        Push-Location ui
        try {
            npm install --no-audit --no-fund
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            npm run build
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        } finally {
            Pop-Location
        }
    }

# Build + copy binary, manifest and (if present) UI bundle into Tabularis's plugin folder.
[linux]
dev-install: build
    mkdir -p ~/.local/share/tabularis/plugins/sqlserver
    cp target/debug/sqlserver-plugin ~/.local/share/tabularis/plugins/sqlserver/
    cp .tabularium ~/.local/share/tabularis/plugins/sqlserver/
    @if [ -f ui/dist/index.js ]; then \
        mkdir -p ~/.local/share/tabularis/plugins/sqlserver/ui/dist; \
        cp ui/dist/index.js ~/.local/share/tabularis/plugins/sqlserver/ui/dist/; \
    fi
    @echo "Installed to ~/.local/share/tabularis/plugins/sqlserver"
    @echo "Restart Tabularis (or toggle the plugin in Settings) to pick up changes."

[macos]
dev-install: build
    mkdir -p "$HOME/Library/Application Support/com.debba.tabularis/plugins/sqlserver"
    cp target/debug/sqlserver-plugin "$HOME/Library/Application Support/tabularis/plugins/sqlserver/"
    cp .tabularium "$HOME/Library/Application Support/tabularis/plugins/sqlserver/"
    @if [ -f ui/dist/index.js ]; then \
        mkdir -p "$HOME/Library/Application Support/tabularis/plugins/sqlserver/ui/dist"; \
        cp ui/dist/index.js "$HOME/Library/Application Support/tabularis/plugins/sqlserver/ui/dist/"; \
    fi
    @echo "Installed to ~/Library/Application Support/com.debba.tabularis/plugins/sqlserver"
    @echo "Restart Tabularis (or toggle the plugin in Settings) to pick up changes."

[windows]
dev-install: build
    $dest = Join-Path $env:APPDATA "debba\tabularis\data\plugins\sqlserver"
    New-Item -ItemType Directory -Force -Path $dest | Out-Null
    Copy-Item "target\debug\sqlserver-plugin.exe" $dest
    Copy-Item ".tabularium" $dest
    if (Test-Path "ui\dist\index.js") {
        New-Item -ItemType Directory -Force -Path (Join-Path $dest "ui\dist") | Out-Null
        Copy-Item "ui\dist\index.js" (Join-Path $dest "ui\dist")
    }
    Write-Host "Installed to $dest"
    Write-Host "Restart Tabularis (or toggle the plugin in Settings) to pick up changes."

# Remove the installed plugin.
[linux]
uninstall:
    rm -rf ~/.local/share/tabularis/plugins/sqlserver

[macos]
uninstall:
    rm -rf "$HOME/Library/Application Support/com.debba.tabularis/plugins/sqlserver"

[windows]
uninstall:
    $dest = Join-Path $env:APPDATA "debba\tabularis\data\plugins\sqlserver"
    if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
