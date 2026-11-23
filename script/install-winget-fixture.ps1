# Installs the pinned WinGet client for the disposable native Windows checks.
$ErrorActionPreference = 'Stop'
if (!$IsWindows -or $env:SKALD_WINGET_TEST_DISPOSABLE -ne '1') {
    throw 'Use a disposable native Windows fixture with SKALD_WINGET_TEST_DISPOSABLE=1.'
}
$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($architecture -notin @('X64', 'Arm64') -or
    [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne $architecture) {
    throw 'WinGet requires native x64 or Arm64 PowerShell for this fixture.'
}

# https://github.com/microsoft/winget-cli/releases/tag/v1.28.240
$version = 'v1.28.240'
$source = "https://github.com/microsoft/winget-cli/releases/download/$version"
$bundle = 'Microsoft.DesktopAppInstaller_8wekyb3d8bbwe.msixbundle'
$dependencies = 'DesktopAppInstaller_Dependencies.zip'
$checksums = @{
    $bundle = '4ea2208f1648462bdc8c211811bbc8b2db1174a4172fb53fdbc52459fe09367e'
    $dependencies = '71a4b86f5fd3d2012e1120fce7937f26b7c7feab0e4302ebd9be79ab0155faa5'
}

$temp = Join-Path ([IO.Path]::GetTempPath()) "skald-winget-client-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $temp | Out-Null
try {
    foreach ($file in $checksums.Keys) {
        $download = Join-Path $temp $file
        Invoke-WebRequest -Uri "$source/$file" -OutFile $download -MaximumRetryCount 3 -RetryIntervalSec 2
        if ((Get-FileHash -LiteralPath $download).Hash -ne $checksums[$file]) {
            throw "SHA-256 mismatch for $file."
        }
    }
    $packages = Join-Path $temp 'packages'
    Expand-Archive -LiteralPath (Join-Path $temp $dependencies) -DestinationPath $packages
    $nativeDependencies = @(Get-ChildItem -LiteralPath (Join-Path $packages $architecture.ToString().ToLowerInvariant()) -Filter '*.appx' -File)
    if (!$nativeDependencies.Count) { throw "The WinGet client has no $architecture dependencies." }
    Add-AppxPackage -Path (Join-Path $temp $bundle) -DependencyPath $nativeDependencies.FullName -ForceUpdateFromAnyVersion

    $aliases = Join-Path $env:LOCALAPPDATA 'Microsoft/WindowsApps'
    $env:Path = "$aliases;$env:Path"
    $winget = Join-Path $aliases 'winget.exe'
    $actual = & $winget --version
    if ($LASTEXITCODE -ne 0 -or $actual -cne $version) { throw "Expected WinGet $version, got $actual." }
    if ($env:GITHUB_PATH) { Add-Content -LiteralPath $env:GITHUB_PATH -Value $aliases }

    # Each scenario serves different bytes at the same URL. Use WinINet for the loopback fixture.
    $state = Join-Path $env:LOCALAPPDATA 'Packages/Microsoft.DesktopAppInstaller_8wekyb3d8bbwe/LocalState'
    New-Item -ItemType Directory -Path $state -Force | Out-Null
    '{"network":{"downloader":"wininet"}}' | Set-Content -LiteralPath (Join-Path $state 'settings.json')
} finally {
    Remove-Item -LiteralPath $temp -Recurse -Force
}
