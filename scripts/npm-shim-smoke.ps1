param(
    [string]$Binary = 'target/x86_64-pc-windows-msvc/debug/ccsw.exe',
    [string]$Helper = 'target/x86_64-pc-windows-msvc/debug/ccsw-test-helper.exe'
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$Binary = (Resolve-Path -LiteralPath $Binary).Path
$Helper = (Resolve-Path -LiteralPath $Helper).Path
$root = Join-Path ([IO.Path]::GetTempPath()) ('ccsw npm 用户 ' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $root | Out-Null
$original = @{}
$values = @{
    HOME = $root; USERPROFILE = $root; APPDATA = "$root\roaming"; LOCALAPPDATA = "$root\local"
    CCSW_CONFIG = "$root\config.toml"; XDG_CONFIG_HOME = "$root\config-root"
    XDG_STATE_HOME = "$root\state"; XDG_CACHE_HOME = "$root\cache"
    CLAUDE_CONFIG_DIR = "$root\claude"; CODEX_HOME = "$root\codex"; PI_CODING_AGENT_DIR = "$root\pi"
    CCSW_TEST_HELPER = $Helper; CCSW_HELPER_RECORD = "$root\args.json"
    CCSW_CLAUDE_BIN = 'ccsw-fixture-claude'; PATH = "$root\node_modules\.bin;$env:PATH"
}
try {
    & npm.cmd install --prefix $root --offline --ignore-scripts --no-audit --no-fund --package-lock=false ./tests/fixtures/npm-client
    if ($LASTEXITCODE -ne 0) { throw 'Cannot generate the npm test shim' }
    foreach ($key in $values.Keys) {
        $original[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
        [Environment]::SetEnvironmentVariable($key, $values[$key], 'Process')
    }
    $output = & $Binary doctor 2>&1
    # No profile is configured: doctor may report that, but the client check must pass.
    if (($output -join "`n") -notmatch 'Claude Code 2\.1\.242') { throw "npm shim failed: $output" }
    $record = Get-Content -LiteralPath "$root\args.json" -Raw | ConvertFrom-Json
    if ($record.args.Count -ne 1 -or $record.args[0] -ne '--version') { throw 'npm shim changed arguments' }
    Write-Host 'Real npm .cmd shim smoke passed'
}
finally {
    foreach ($key in $original.Keys) { [Environment]::SetEnvironmentVariable($key, $original[$key], 'Process') }
    Remove-Item -LiteralPath $root -Recurse -Force
}

# Expected diagnostic/cleanup failures must not become the runner exit status.
$global:LASTEXITCODE = 0
