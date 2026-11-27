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

    # The semantic search model, which the archive carries and the binary loads from
    # `model\` beside it. Without it the binary still works and searches by words
    # only, so a failure here is a warning and not a failed install.
    if (Test-Path "$temp\$package\model") {
        try {
            New-Item -ItemType Directory -Force -Path (Join-Path $installDir 'model') | Out-Null
            Copy-Item "$temp\$package\model\*" (Join-Path $installDir 'model') -Force
        } catch {
            Write-Warning "could not install the search model: $_; run 'leteo model install' later"
        }
    }

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

    # The PATH entry, read and written as the registry holds it.
    #
    # `[Environment]::GetEnvironmentVariable('Path','User')` hands back every
    # `%VAR%` already expanded, and `SetEnvironmentVariable` stores the result
    # as a plain string. Measured on `windows-latest` under both PowerShell
    # engines: `%USERPROFILE%\.cargo\bin;...` came back as
    # `C:\Users\runneradmin\.cargo\bin;...` and the value's kind went from
    # `ExpandString` to `String`, so every entry that names a variable stopped
    # following it. Leteo owns one entry on this value; the rest are read with
    # `DoNotExpandEnvironmentNames` and written back with the kind they had.
    #
    # The five functions below are the same five `uninstall.ps1` carries, in the
    # same order and shape. The two scripts cannot share a file — each is
    # fetched and run on its own (`irm ... | iex` here, and the uninstaller
    # ships beside the binary) — and `tests/script_parity.rs` compares both
    # copies line for line, a line that is only a comment aside, failing when
    # one has changed and the other has not.
    $onWindows = ($env:OS -eq 'Windows_NT')
    $environment = $null
    $openError = $null
    try {
        $environment = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    } catch {
        $environment = $null
        $openError = $_.Exception.Message
    }
    if ($onWindows -and -not $environment) {
        # Loud, because the fallback is the API that expands every `%VAR%` and
        # stores plain text: the rest of the PATH can be frozen by it.
        if ($openError) {
            Write-Warning "could not open HKCU:\Environment ($openError); the rest of your PATH may have its variables frozen"
        } else {
            # `OpenSubKey` returns null, rather than throwing, when the key does
            # not exist: a machine that never set a user variable has none.
            Write-Warning 'HKCU:\Environment does not exist; the Path Leteo writes is plain text'
        }
    }

    function Read-UserPath {
        if ($environment) {
            return [string]$environment.GetValue(
                'Path',
                '',
                [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
            )
        }
        [string][Environment]::GetEnvironmentVariable('Path', 'User')
    }

    # Tells running programs the environment changed.
    #
    # `[Environment]::SetEnvironmentVariable(..., 'User')`, which this replaced,
    # broadcasts `WM_SETTINGCHANGE` for `Environment` after writing. A
    # `RegistryKey.SetValue` does not, so Explorer and every terminal opened
    # from it would keep the PATH they started with until the next logon —
    # while the installer says to open a new terminal.
    #
    # Everything here is caught: the value is already written when this runs, so
    # a host that refuses `Add-Type` must not turn a successful install into a
    # failure. What it costs is a warning and a closing line that asks for a new
    # session rather than a new terminal.
    function Send-EnvironmentChange {
        try {
            if (-not ('Leteo.EnvironmentBroadcast' -as [type])) {
                Add-Type -Namespace Leteo -Name EnvironmentBroadcast -MemberDefinition @'
[DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
public static extern System.IntPtr SendMessageTimeout(
    System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam,
    uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);
'@
            }
            $broadcast = [IntPtr]0xffff          # HWND_BROADCAST
            $settingChange = 0x001A           # WM_SETTINGCHANGE
            $abortIfHung = 0x0002             # SMTO_ABORTIFHUNG
            $timeoutMs = 5000
            $result = [UIntPtr]::Zero
            $sent = [Leteo.EnvironmentBroadcast]::SendMessageTimeout(
                $broadcast, $settingChange, [UIntPtr]::Zero, 'Environment', $abortIfHung, $timeoutMs, [ref]$result
            )
            if ($sent -eq [IntPtr]::Zero) {
                Write-Warning 'the environment change could not be broadcast; start a new session rather than a new terminal'
                return $false
            }
            return $true
        } catch {
            Write-Warning "the environment change could not be broadcast ($($_.Exception.Message)); start a new session rather than a new terminal"
            return $false
        }
    }

    # Writes the value, and returns whether the change reached running programs:
    # the closing advice is the one thing that depends on the answer.
    function Write-UserPath([string]$Value) {
        if ($environment) {
            $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
            # Asked rather than caught: a failure to read the kind of a value
            # that is there is not the same thing as the value being absent,
            # and only the absent case has a default.
            if ($null -ne $environment.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)) {
                $kind = $environment.GetValueKind('Path')
            }
            $environment.SetValue('Path', $Value, $kind)
            return Send-EnvironmentChange
        }
        [Environment]::SetEnvironmentVariable('Path', $Value, 'User')
        return $true
    }

    # One spelling for a PATH entry: variables expanded, separators folded, a
    # trailing one dropped. Windows accepts `C:/dir` for `C:\dir`, and an entry
    # names the same directory whatever it ends with.
    function Get-ComparablePath([string]$Entry) {
        [Environment]::ExpandEnvironmentVariables($Entry).Replace('/', '\').TrimEnd('\')
    }

    # An entry that names the directory through a variable, or with a separator
    # spelled differently, is that entry: matching only the exact text would add
    # a second copy of a directory already on the PATH. What gets written stays
    # exactly as it was read.
    function Test-SamePathEntry([string]$Entry) {
        if (-not $Entry) { return $false }
        (Get-ComparablePath $Entry) -eq (Get-ComparablePath $installDir)
    }

    $missing = $false
    $notified = $true
    try {
        $userPath = Read-UserPath
        # The value is written back exactly as it was read, empty entries
        # included: they are the person's, and rewriting text Leteo does not own
        # is the defect this whole block exists to avoid. The only `;` written is
        # the one between their value and the entry; an empty value gets the
        # entry alone.
        $entries = @($userPath -split ';')
        $missing = -not ($entries | Where-Object { Test-SamePathEntry $_ })
        if ($missing) {
            $value = if ($userPath) { "$userPath;$installDir" } else { $installDir }
            $notified = Write-UserPath $value
        }
    } finally {
        if ($environment) {
            $environment.Close()
        }
    }
    Write-Host ""
    if ($missing) {
        Write-Host "Installed, and $installDir was added to your PATH."
        if ($notified) {
            Write-Host "Open a new terminal, then run 'leteo setup' to configure your agents."
        } else {
            # The broadcast that tells Explorer the environment changed did not
            # go out, so a terminal opened from the menu would still lack the
            # entry — the advice has to be the one that works.
            Write-Host "Sign out and back in, then run 'leteo setup' to configure your agents."
        }
    } else {
        Write-Host "Installed. Run 'leteo setup' to configure your agents."
    }
    Write-Host "To remove it: 'leteo uninstall', or Leteo in Settings > Installed apps."
} finally {
    Remove-Item -Recurse -Force $temp -ErrorAction SilentlyContinue
}
