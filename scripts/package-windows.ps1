param(
    [string]$Binary = 'target/x86_64-pc-windows-msvc/release/ccsw.exe',
    [string]$OutputDirectory = 'target/dist'
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$Binary = (Resolve-Path -LiteralPath $Binary).Path
$version = & $Binary --version
if ($LASTEXITCODE -ne 0) { throw 'Release binary cannot run' }
$commit = git rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Cannot identify source commit' }
$compiler = rustc --version
if ($LASTEXITCODE -ne 0) { throw 'Cannot identify compiler' }
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or -not $installation) { throw 'MSVC tools unavailable' }
$dumpbin = Get-ChildItem "$installation\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $dumpbin) { throw 'dumpbin unavailable' }
$imports = & $dumpbin.FullName /DEPENDENTS $Binary
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect PE imports' }
if (($imports -join "`n") -match '(?i)(vcruntime\d*|msvcp\d+|libgcc[^\s]*|libstdc\+\+[^\s]*|libwinpthread[^\s]*|ucrtbase)\.dll') {
    throw 'Release unexpectedly requires a separately installed C/C++ runtime'
}
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$OutputDirectory = (Resolve-Path $OutputDirectory).Path
$stage = Join-Path ([IO.Path]::GetTempPath()) ('ccsw-package-' + [guid]::NewGuid())
$extract = Join-Path ([IO.Path]::GetTempPath()) ('ccsw 解压 package ' + [guid]::NewGuid())
try {
    New-Item -ItemType Directory -Path $stage | Out-Null
    Copy-Item -LiteralPath $Binary -Destination "$stage\ccsw.exe"
    Copy-Item -LiteralPath 'LICENSE' -Destination "$stage\LICENSE"
    $readme = Get-Content -LiteralPath 'README-Windows.md' -Raw -Encoding utf8
    $readme += "`n`nBuild: $version`nCommit: $commit`nTarget: x86_64-pc-windows-msvc`nCompiler: $compiler`nVerification: native Windows packaging smoke; full regression results are reported separately.`n"
    Set-Content -LiteralPath "$stage\README-Windows.md" -Value $readme -Encoding utf8
    $zip = Join-Path $OutputDirectory 'ccsw-windows-x86_64.zip'
    Compress-Archive -LiteralPath "$stage\ccsw.exe", "$stage\README-Windows.md", "$stage\LICENSE" -DestinationPath $zip -Force
    Expand-Archive -LiteralPath $zip -DestinationPath $extract
    & "$PSScriptRoot/windows-smoke.ps1" -Binary "$extract\ccsw.exe"
    $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath "$zip.sha256" -Value "$hash  ccsw-windows-x86_64.zip" -Encoding ascii
    Write-Host "Packaged and verified: $zip"
}
finally {
    foreach ($path in @($stage, $extract)) { if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Recurse -Force } }
}

# Expected diagnostic/cleanup failures must not become the runner exit status.
$global:LASTEXITCODE = 0
