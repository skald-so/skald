# Run as an administrator on a disposable Windows fixture with WinGet and no skald installation.
# Both architectures consume the same generated manifest directory and the same fixture port.
param(
    [Parameter(Mandatory)][string]$ManifestDir,
    [Parameter(Mandatory)][string]$ReleaseDir,
    [Parameter(Mandatory)][string]$Tag,
    [Parameter(Mandatory)]
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')]
    [string]$Target,
    [Parameter(Mandatory)][ValidateRange(1024, 65535)][int]$Port
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

if (!$IsWindows -or $env:SKALD_WINGET_TEST_DISPOSABLE -ne '1') {
    throw 'Use a disposable native Windows fixture with SKALD_WINGET_TEST_DISPOSABLE=1.'
}
if ($Tag -cnotmatch '^v\d+\.\d+\.\d+$') { throw "Expected a stable release tag, got $Tag." }
$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
$targets = @{ X64 = 'x86_64-pc-windows-msvc'; Arm64 = 'aarch64-pc-windows-msvc' }
if ($Target -ne $targets[$architecture.ToString()] -or [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne $architecture) {
    throw "$Target requires a native Windows runner and PowerShell process."
}
$winget = (Get-Command winget.exe -CommandType Application).Source
$node = (Get-Command node.exe -CommandType Application).Source
$shell = [Environment]::ProcessPath
$version = $Tag.Substring(1)
$manifest = @(Get-ChildItem -LiteralPath $ManifestDir -Filter '*.installer.yaml' -File)
if ($manifest.Count -ne 1) { throw 'Expected one installer manifest.' }
$archive = "skald-client-$Target.zip"
$otherTarget = $targets.Values | Where-Object { $_ -ne $Target }
$otherArchive = "skald-client-$otherTarget.zip"
$base = "http://127.0.0.1:$Port"
$urls = @(Select-String -LiteralPath $manifest[0].FullName -Pattern '^\s*(?:-\s*)?InstallerUrl:\s*["'']?(?<url>[^\s"'']+)["'']?\s*$' |
    ForEach-Object { $_.Matches[0].Groups['url'].Value })
if ($urls.Count -ne 2 -or $urls -cnotcontains "$base/$archive" -or $urls -cnotcontains "$base/$otherArchive") {
    throw 'Both manifest entries must point to the two local release archives.'
}

$temp = Join-Path ([IO.Path]::GetTempPath()) "skald-winget-$([guid]::NewGuid())"
$mirror = New-Item -ItemType Directory -Path (Join-Path $temp 'archives')
$probe = Join-Path $temp 'new-shell.ps1'
$requests = Join-Path $temp 'requests.log'
$serverErrors = Join-Path $temp 'server.log'
$server = $null
try {
    @'
$ErrorActionPreference = 'Stop'
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
    [Environment]::GetEnvironmentVariable('Path', 'User')
$skald = Get-Command skald -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if (!$skald) { exit 3 }
& skald --version
if ($LASTEXITCODE -ne 0) { throw 'skald --version failed.' }
(Get-FileHash -LiteralPath $skald.Source).Hash
'@ | Set-Content -LiteralPath $probe

    function Assert-SkaldAbsent {
        & $shell -NoProfile -NonInteractive -File $probe
        if ($LASTEXITCODE -ne 3) { throw 'The fixture shell must have no skald installation.' }
    }
    Assert-SkaldAbsent
    & $winget settings --enable LocalManifestFiles
    if ($LASTEXITCODE -ne 0) { throw 'WinGet could not enable local manifests.' }
    & $winget validate --manifest $ManifestDir
    if ($LASTEXITCODE -ne 0) { throw 'WinGet could not validate the manifest set.' }

    Copy-Item -LiteralPath (Join-Path $ReleaseDir $otherArchive) -Destination $mirror.FullName
    $serverScript = Join-Path $PSScriptRoot 'lib/serve.mjs'
    $serverArgs = ('"{0}" "{1}"' -f $serverScript, $mirror.FullName)
    $server = Start-Process -FilePath $node -ArgumentList $serverArgs -PassThru -NoNewWindow `
        -Environment @{ SKALD_TEST_RELEASE_PORT = "$Port" } -RedirectStandardOutput $requests -RedirectStandardError $serverErrors
    for ($attempt = 0; $attempt -lt 50 -and !(Get-Content -LiteralPath $requests); $attempt++) {
        if ($server.HasExited) { throw (Get-Content -LiteralPath $serverErrors -Raw) }
        Start-Sleep -Milliseconds 100
    }
    if ((Get-Content -LiteralPath $requests -TotalCount 1) -ne "$Port") { throw 'The fixture archive server did not start.' }

    $nativeZip = Join-Path $ReleaseDir $archive
    $servedArchive = Join-Path $mirror.FullName $archive
    foreach ($scenario in @('missing', 'mismatched', 'valid')) {
        switch ($scenario) {
            'mismatched' { Copy-Item -LiteralPath (Join-Path $ReleaseDir $otherArchive) -Destination $servedArchive }
            'valid' { Copy-Item -LiteralPath $nativeZip -Destination $servedArchive -Force }
        }
        $before = @(Get-Content -LiteralPath $requests).Count
        # Both entries are present, and WinGet selects the native one without an architecture flag.
        & $winget install --manifest $ManifestDir --scope user --skip-dependencies --disable-interactivity --accept-package-agreements
        $code = $LASTEXITCODE
        $downloads = @(Get-Content -LiteralPath $requests | Select-Object -Skip $before)
        if ($downloads -cnotcontains "/$archive" -or $downloads -ccontains "/$otherArchive") {
            throw "WinGet did not select the native archive for $scenario. Requests were $downloads."
        }
        if ($scenario -eq 'valid') {
            if ($code -ne 0) { throw "WinGet install failed with $code." }
        } else {
            if ($code -eq 0) { throw "WinGet installed with a $scenario archive." }
            # APPINSTALLER_CLI_ERROR_INSTALLER_HASH_MISMATCH.
            if ($scenario -eq 'mismatched' -and $code -ne -1978335215) {
                throw "WinGet returned $code for the wrong checksum."
            }
            Assert-SkaldAbsent
        }
    }

    $expanded = Join-Path $temp 'expected'
    Expand-Archive -LiteralPath $nativeZip -DestinationPath $expanded
    $hash = (Get-FileHash -LiteralPath (Join-Path $expanded 'skald.exe')).Hash
    $actual = @(& $shell -NoProfile -NonInteractive -File $probe)
    if ($LASTEXITCODE -ne 0 -or ($actual -join "`n") -cne "skald $version`n$hash") {
        throw "A fresh shell did not run the exact $Target release binary. Output was $actual."
    }
    Write-Output "WinGet installed skald $version from $archive."
} finally {
    if ($server -and !$server.HasExited) { Stop-Process -Id $server.Id }
    Remove-Item -LiteralPath $temp -Recurse -Force
}
