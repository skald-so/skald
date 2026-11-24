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

# The package identity and framework minimums are from the native payload manifests.
# https://github.com/microsoft/winget-cli/releases/tag/v1.29.380
# This release includes https://github.com/microsoft/winget-cli/pull/6127 for local installer scans.
$version = 'v1.29.380'
$packageVersion = [version]'1.29.380.0'
$publisher = 'CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US'
$nativeArchitecture = $architecture.ToString().ToLowerInvariant()
$appInstaller = Get-AppxPackage -Name 'Microsoft.DesktopAppInstaller' -Publisher $publisher -PackageTypeFilter Main |
    Where-Object { $_.Architecture -eq $nativeArchitecture -and [version]$_.Version -eq $packageVersion -and $_.Status -eq 'Ok' } |
    Select-Object -First 1

if (!$appInstaller) {
    $source = "https://github.com/microsoft/winget-cli/releases/download/$version"
    $bundle = 'Microsoft.DesktopAppInstaller_8wekyb3d8bbwe.msixbundle'
    $dependencies = 'DesktopAppInstaller_Dependencies.zip'
    $checksums = @{
        $bundle = '65dea9c01ce08ee7b763366b27c0e651f97db857c11ca9b9c301826c10092f2e'
        $dependencies = 'ba875afe9d190f61218985ac0292a99d1db710bf93e13c68944ca9d89f0d82d1'
    }
    $minimumVersions = @{
        'Microsoft.VCLibs.140.00' = [version]'14.0.33519.0'
        'Microsoft.VCLibs.140.00.UWPDesktop' = [version]'14.0.33728.0'
        'Microsoft.WindowsAppRuntime.1.8' = [version]'8000.616.304.0'
    }
    $installed = @(Get-AppxPackage -Publisher $publisher -PackageTypeFilter Framework |
        Where-Object { $_.Architecture -eq $nativeArchitecture })
    # A compatible framework can be in use by Windows. Leave it out of the deployment.
    $missing = @(foreach ($name in $minimumVersions.Keys) {
        $compatible = $installed | Where-Object { $_.Name -eq $name -and [version]$_.Version -ge $minimumVersions[$name] }
        if (!$compatible) { $name }
    })

    $temp = Join-Path ([IO.Path]::GetTempPath()) "skald-winget-client-$([guid]::NewGuid())"
    New-Item -ItemType Directory -Path $temp | Out-Null
    try {
        $assets = @($bundle)
        if ($missing.Count) { $assets += $dependencies }
        foreach ($file in $assets) {
            $download = Join-Path $temp $file
            Invoke-WebRequest -Uri "$source/$file" -OutFile $download -MaximumRetryCount 3 -RetryIntervalSec 2
            if ((Get-FileHash -LiteralPath $download).Hash -ne $checksums[$file]) {
                throw "SHA-256 mismatch for $file."
            }
        }
        $install = @{ Path = (Join-Path $temp $bundle); ForceUpdateFromAnyVersion = $true }
        if ($missing.Count) {
            $packages = Join-Path $temp 'packages'
            Expand-Archive -LiteralPath (Join-Path $temp $dependencies) -DestinationPath $packages
            $install.DependencyPath = @(foreach ($name in $missing) {
                $minimum = $minimumVersions[$name]
                $file = Join-Path $packages $nativeArchitecture "${name}_${minimum}_$nativeArchitecture.appx"
                if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing WinGet framework package $file." }
                $file
            })
        }
        Add-AppxPackage @install
    } finally {
        Remove-Item -LiteralPath $temp -Recurse -Force
    }
} else {
    Write-Output "Reusing WinGet $version on $nativeArchitecture."
}

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
