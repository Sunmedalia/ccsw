param([Parameter(Mandatory = $true)][string]$Binary)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$Binary = (Resolve-Path -LiteralPath $Binary).Path
$root = Join-Path ([IO.Path]::GetTempPath()) ('ccsw 验证 & %literal% !^() ' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $root | Out-Null
$values = @{
    HOME = $root; USERPROFILE = $root; APPDATA = "$root\roaming"; LOCALAPPDATA = "$root\local"
    CCSW_CONFIG = "$root\config.toml"; XDG_CONFIG_HOME = "$root\config-root"
    XDG_STATE_HOME = "$root\state"; XDG_CACHE_HOME = "$root\cache"
    CLAUDE_CONFIG_DIR = "$root\claude"; CODEX_HOME = "$root\codex"; PI_CODING_AGENT_DIR = "$root\pi"
    ANTHROPIC_API_KEY = $null; ANTHROPIC_AUTH_TOKEN = $null; ANTHROPIC_BASE_URL = $null
    OPENAI_API_KEY = $null; CODEX_ACCESS_TOKEN = $null; CODEX_AUTH = $null
}
$original = @{}
function Invoke-Ccsw([string[]]$Arguments) {
    $result = & $Binary @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw "ccsw $($Arguments -join ' ') failed: $result" }
    return ($result -join "`n")
}
try {
    foreach ($key in $values.Keys) {
        $original[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
        [Environment]::SetEnvironmentVariable($key, $values[$key], 'Process')
    }
    Invoke-Ccsw -Arguments @('--help') | Out-Null
    $version = Invoke-Ccsw -Arguments @('--version')
    if ($version -notmatch '^ccsw \d+\.\d+\.\d+') { throw "Unexpected version: $version" }
    $path = Invoke-Ccsw -Arguments @('config', 'path')
    if ($path.Trim() -ne "$root\config.toml") { throw "Wrong config path: $path" }
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $listener.Stop()
    Invoke-Ccsw -Arguments @('proxy', 'port', "$port") | Out-Null
    Invoke-Ccsw -Arguments @('proxy', 'start') | Out-Null
    # A second invocation must return promptly while the detached daemon is alive.
    Invoke-Ccsw -Arguments @('proxy', 'status') | Out-Null
    Invoke-Ccsw -Arguments @('proxy', 'stop') | Out-Null
    Invoke-Ccsw -Arguments @('proxy', 'start') | Out-Null
    Invoke-Ccsw -Arguments @('proxy', 'stop') | Out-Null
    # Exercise client preference sync and restoration using the packaged EXE.
    # This does not call an external model or require API credentials.
    New-Item -ItemType Directory -Force -Path "$root\claude" | Out-Null
    $settings = "$root\claude\settings.json"
    '{"env":{"CCSW_SMOKE_LITERAL":"before","KEEP":"yes"},"attribution":{"commit":"original","pr":"original-pr"}}' | Set-Content -LiteralPath $settings -Encoding utf8NoBOM
    @'
version = 5
[claude]
hide_attribution = true
[claude.env]
CCSW_SMOKE_LITERAL = 'literal %PATH% !VALUE! & ^ ( ) $value'
CLAUDE_CODE_EFFORT_LEVEL = 'max'
[profiles.smoke]
name = 'Windows smoke'
base_url = 'https://example.invalid'
default_model = 'smoke-model[1m]'
[[profiles.smoke.models]]
id = 'smoke-model[1m]'
reasoning_max = 'high'
'@ | Set-Content -LiteralPath "$root\config.toml" -Encoding utf8NoBOM
    Invoke-Ccsw -Arguments @('apply', '--profile', 'smoke') | Out-Null
    $applied = Get-Content -LiteralPath $settings -Raw | ConvertFrom-Json
    if ($applied.env.CCSW_SMOKE_LITERAL -cne 'literal %PATH% !VALUE! & ^ ( ) $value') { throw 'Literal environment value changed' }
    if ($applied.env.CLAUDE_CODE_EFFORT_LEVEL -ne 'max') { throw 'Effort setting missing' }
    if ($applied.attribution.commit -ne '' -or $applied.attribution.pr -ne '') { throw 'Attribution not hidden' }
    Invoke-Ccsw -Arguments @('apply', '--profile', 'smoke') | Out-Null
    Invoke-Ccsw -Arguments @('uninstall', '--yes') | Out-Null
    $restored = Get-Content -LiteralPath $settings -Raw | ConvertFrom-Json
    if ($restored.env.CCSW_SMOKE_LITERAL -ne 'before' -or $restored.env.KEEP -ne 'yes') { throw 'Environment baseline not restored' }
    if ($restored.attribution.commit -ne 'original' -or $restored.attribution.pr -ne 'original-pr') { throw 'Attribution baseline not restored' }
    if ($null -ne $restored.env.CLAUDE_CODE_EFFORT_LEVEL) { throw 'Owned effort setting was not removed' }
    Invoke-Ccsw -Arguments @('uninstall', '--yes') | Out-Null
    Write-Host "Windows extracted-binary smoke passed: $version"
}
finally {
    # Stop only the isolated instance, including on a failed assertion.
    & $Binary proxy stop 2>&1 | Out-Null
    foreach ($key in $original.Keys) { [Environment]::SetEnvironmentVariable($key, $original[$key], 'Process') }
    Remove-Item -LiteralPath $root -Recurse -Force
}

# Expected diagnostic/cleanup failures must not become the runner exit status.
$global:LASTEXITCODE = 0
