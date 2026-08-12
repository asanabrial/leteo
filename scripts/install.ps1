# Installs Leteo on Windows.
#
#   irm https://raw.githubusercontent.com/asanabrial/leteo/main/scripts/install.ps1 | iex
#
# Downloads the release archive for this machine, checks it against the
# published SHA-256 sums, and puts the binary on the user's PATH. Nothing is
# installed if the checksum does not match.

$ErrorActionPreference = 'Stop'

# Windows PowerShell 5.1 is what Windows ships, and it does not negotiate TLS
# 1.2 unless told to. GitHub refuses anything older, so without this the very
# first download fails on a default machine.
try {
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch {
    # PowerShell 7 manages this itself and may not expose the property.
}

$repo = 'asanabrial/leteo'
$installDir = if ($env:LETEO_INSTALL_DIR) { $env:LETEO_INSTALL_DIR } else { "$env:LOCALAPPDATA\leteo\bin" }
$version = if ($env:LETEO_VERSION) { $env:LETEO_VERSION } else { 'latest' }

function Fail($message) {
    Write-Error "error: $message"
    exit 1
}

$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'x86_64' }
    default { Fail "no prebuilt Leteo for $($env:PROCESSOR_ARCHITECTURE); build from source with 'cargo install --git https://github.com/$repo'" }
}
$target = "$arch-pc-windows-msvc"

if ($version -eq 'latest') {
    # The redirect from /releases/latest names the tag, which avoids depending
    # on the API and its rate limit.
    #
    # Read through .NET rather than Invoke-WebRequest: the switches for holding
    # a redirect differ between Windows PowerShell 5.1 and PowerShell 7, and
    # this has to work on the one already installed.
    $location = $null
    try {
        $request = [System.Net.WebRequest]::Create("https://github.com/$repo/releases/latest")
        $request.Method = 'HEAD'
        $request.AllowAutoRedirect = $false
        $response = $request.GetResponse()
        $location = $response.Headers['Location']
        $response.Close()
    } catch [System.Net.WebException] {
        # A 404 means there is nothing published yet, which is a different
        # problem from a network that is down, and sends the reader somewhere
        # else entirely.
        $status = $_.Exception.Response.StatusCode.value__
        if ($status -eq 404) {
            Fail "$repo has no published releases yet; build from source with 'cargo install --git https://github.com/$repo'"
        }
        Fail "could not reach GitHub to find the latest version ($status); set LETEO_VERSION"
    } catch {
        Fail "could not reach GitHub to find the latest version; set LETEO_VERSION"
    }
    if (-not $location) { Fail "could not work out the latest version; set LETEO_VERSION" }
    $version = ($location -split '/tag/')[-1]
}

$package = "leteo-$version-$target"
$archive = "$package.zip"
# Overridable so an internal mirror can serve the same layout, and so the
# verification path can be exercised without publishing anything.
$base = if ($env:LETEO_BASE_URL) { $env:LETEO_BASE_URL } else { "https://github.com/$repo/releases/download/$version" }

Write-Host "Leteo $version for $target"

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("leteo-install-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $temp | Out-Null
try {
    Write-Host "  downloading"
    try {
        Invoke-WebRequest -Uri "$base/$archive" -OutFile "$temp\$archive"
        Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile "$temp\SHA256SUMS"
    } catch {
        Fail "no release archive at $base/$archive"
    }

    Write-Host "  verifying"
    $line = Get-Content "$temp\SHA256SUMS" | Where-Object { $_ -match "\s\*?$([regex]::Escape($archive))$" } | Select-Object -First 1
    if (-not $line) { Fail "$archive is not listed in SHA256SUMS" }
    $expected = ($line -split '\s+')[0]
    $actual = (Get-FileHash "$temp\$archive" -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $expected.ToLower()) {
        Fail "checksum mismatch: expected $expected, got $actual"
    }

    Write-Host "  installing to $installDir"
    Expand-Archive -Path "$temp\$archive" -DestinationPath $temp -Force
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    Copy-Item "$temp\$package\leteo.exe" (Join-Path $installDir 'leteo.exe') -Force

    # The uninstaller ships beside the binary rather than being downloaded when
    # it is needed: somebody removing a tool should not have to be online, and
    # `leteo.exe` cannot delete itself on Windows — a separate script is the
    # only thing that can.
    #
    # Loud when it is missing rather than skipped. A release archive built
    # without it still installs perfectly, and the only visible consequence is
    # that Leteo never appears in Installed apps — which reads as a decision
    # somebody made rather than as a packaging mistake, and would sit there for
    # releases.
    $stagedUninstaller = "$temp\$package\uninstall.ps1"
    if (Test-Path $stagedUninstaller) {
        Copy-Item $stagedUninstaller (Join-Path $installDir 'uninstall.ps1') -Force
    } else {
        Write-Warning "this archive ships no uninstall.ps1, so Leteo will not appear in Installed apps; 'leteo uninstall' still removes everything but the binary"
    }

    # What puts Leteo in Settings > Installed apps.
    #
    # There is no installer database on Windows and no wizard the system
    # provides: that list is built by reading these keys, and pressing Uninstall
    # runs whatever `UninstallString` says. Without this, a tool dropped on the
    # PATH by a one-line command is invisible to every place a person looks for
    # the programs on their machine.
    #
    # HKCU rather than HKLM because this installer never asks for administrator,
    # so the entry belongs to the account that installed it and to no other.
    #
    # `QuietUninstallString` is what Settings prefers, and it is the one that
    # must not prompt: Windows may run it with no console attached, where a
    # confirmation nobody can see is a hang rather than a question.
    $uninstaller = Join-Path $installDir 'uninstall.ps1'
    $registryKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Leteo'
    if (Test-Path $uninstaller) {
        New-Item -Path $registryKey -Force | Out-Null
        $run = "powershell -NoProfile -ExecutionPolicy Bypass -File `"$uninstaller`""
        $size = [math]::Round((Get-Item (Join-Path $installDir 'leteo.exe')).Length / 1KB)
        Set-ItemProperty -Path $registryKey -Name 'DisplayName'          -Value 'Leteo'
        Set-ItemProperty -Path $registryKey -Name 'DisplayVersion'       -Value $version
        Set-ItemProperty -Path $registryKey -Name 'Publisher'            -Value 'Leteo'
        Set-ItemProperty -Path $registryKey -Name 'InstallLocation'      -Value $installDir
        Set-ItemProperty -Path $registryKey -Name 'DisplayIcon'          -Value (Join-Path $installDir 'leteo.exe')
        Set-ItemProperty -Path $registryKey -Name 'UninstallString'      -Value $run
        Set-ItemProperty -Path $registryKey -Name 'QuietUninstallString' -Value "$run -Yes"
        # In KB, which is the unit the list reads it as.
        Set-ItemProperty -Path $registryKey -Name 'EstimatedSize'        -Value $size -Type DWord
        # Neither is offered, so neither button should appear.
        Set-ItemProperty -Path $registryKey -Name 'NoModify'             -Value 1 -Type DWord
        Set-ItemProperty -Path $registryKey -Name 'NoRepair'             -Value 1 -Type DWord
    }

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $installDir) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$installDir", 'User')
        Write-Host ""
        Write-Host "Installed, and $installDir was added to your PATH."
        Write-Host "Open a new terminal, then run 'leteo setup' to configure your agents."
    } else {
        Write-Host ""
        Write-Host "Installed. Run 'leteo setup' to configure your agents."
    }
    Write-Host "To remove it: 'leteo uninstall', or Leteo in Settings > Installed apps."
} finally {
    Remove-Item -Recurse -Force $temp -ErrorAction SilentlyContinue
}
