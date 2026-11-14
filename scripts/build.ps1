$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Install stable Rust from https://rustup.rs and Visual Studio C++ Build Tools, then rerun this script.'
}
cargo fmt
if ($LASTEXITCODE -ne 0) { throw 'Rust formatting failed.' }
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Rust build failed.' }
New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item target/release/clearwave.exe dist/clearwave.exe -Force
Write-Output 'Built dist/clearwave.exe. Double-click it to launch Clearwave.'

