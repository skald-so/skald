# Installs skald from its GitHub Release on Windows: downloads the release's archive for this
# machine's architecture and puts skald.exe in %LOCALAPPDATA%\Programs\skald, or
# $env:SKALD_INSTALL_DIR, on the user's PATH, so new shells run it. script/release-installers sets
# the release tag of a release's copy.
#   powershell -ExecutionPolicy Bypass -c "irm https://github.com/skald-so/skald/releases/download/<tag>/install.ps1 | iex"
# $env:SKALD_DOWNLOAD_URL: where the release's files are instead of its GitHub Release.
# Runs in a script block and throws on errors, so `iex` leaves the shell open and its variables
# unchanged.
& {
  $ErrorActionPreference = 'Stop'
  $ProgressPreference = 'SilentlyContinue'
  $tag = '@RELEASE_TAG@'
  if ($tag -notmatch '^v[0-9]') {
    throw 'install.ps1: this is the template of the installer; run install.ps1 of a release: https://github.com/skald-so/skald/releases'
  }

  # The machine's own architecture, also when PowerShell runs emulated.
  $environment = Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
  switch ($environment.PROCESSOR_ARCHITECTURE) {
    'AMD64' { $target = 'x86_64-pc-windows-msvc' }
    'ARM64' { $target = 'aarch64-pc-windows-msvc' }
    default { throw "install.ps1: no skald for the architecture $($environment.PROCESSOR_ARCHITECTURE)" }
  }
  $archive = "skald-client-$target.zip"
  $base = if ($env:SKALD_DOWNLOAD_URL) { $env:SKALD_DOWNLOAD_URL } else { "https://github.com/skald-so/skald/releases/download/$tag" }
  $dir = if ($env:SKALD_INSTALL_DIR) { $env:SKALD_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\skald' }

  # Windows PowerShell 5.1 does not offer TLS 1.2 by default.
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
  $tmp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
  New-Item -ItemType Directory -Path $tmp | Out-Null
  try {
    $zip = Join-Path $tmp $archive
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$archive" -OutFile $zip
    Expand-Archive -Path $zip -DestinationPath (Join-Path $tmp 'archive')
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Copy-Item -Force -Path (Join-Path $tmp 'archive\skald.exe') -Destination (Join-Path $dir 'skald.exe')
  } finally {
    Remove-Item -Recurse -Force -Path $tmp
  }
  Write-Output "install.ps1: installed skald $tag in $dir"

  # The user's PATH in the registry, its %VARIABLES% unexpanded: new shells read it from there.
  $key = Get-Item -Path 'HKCU:\Environment'
  $path = $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
  $entries = @($path -split ';' | Where-Object { $_ })
  if ($entries -notcontains $dir) {
    Set-ItemProperty -Path 'HKCU:\Environment' -Name 'Path' -Type ExpandString -Value (($entries + $dir) -join ';')
    # Setting a user variable through .NET tells running programs, such as Explorer, that the
    # environment changed.
    [Environment]::SetEnvironmentVariable('SKALD_INSTALL_NOTIFY', '1', 'User')
    [Environment]::SetEnvironmentVariable('SKALD_INSTALL_NOTIFY', $null, 'User')
    Write-Output "install.ps1: added $dir to the user's PATH; open a new shell to run skald"
  }
  if (@($env:Path -split ';') -notcontains $dir) { $env:Path = "$env:Path;$dir" }
}
