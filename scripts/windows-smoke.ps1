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
    Invoke-Ccsw -Arguments @('uninstall', '--yes') | Out-Null
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
