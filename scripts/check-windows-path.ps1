# Checks that installing and uninstalling Leteo leaves the rest of the user PATH
# exactly as it was.
#
# A PATH entry spelled with a variable — `%USERPROFILE%\bin` — is stored as a
# `REG_EXPAND_SZ` value, and Windows expands it when a program asks for the
# value as text. Writing that text back stores the expansion, so the entry is
# frozen at whatever the variable meant at that moment: a user whose profile
# moves, or whose entry names a variable they later change, finds an entry that
# no longer follows it. Leteo owns one entry on that value and nothing else on
# it.
#
# Run by the windows legs of `ci.yml`, once under Windows PowerShell 5.1
# (`shell: powershell`) and once under PowerShell 7 (`shell: pwsh`), because the
# two engines read the value differently.
#
# Needs a built `leteo.exe` (the checkout's `target\debug` by default) and a
# release archive the installer can fetch: the script assembles both and serves
# them from loopback, so it exercises the real `install.ps1` rather than a copy
# of its PATH logic.

[CmdletBinding()]
param(
    [string]$Binary = (Join-Path $PSScriptRoot '..\target\debug\leteo.exe')
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
    throw "no leteo.exe at $Binary; run 'cargo build' first"
}

$version = 'v0.0.0-path-check'
$package = "leteo-$version-x86_64-pc-windows-msvc"
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ('leteo-path-check-' + [guid]::NewGuid())
$dist = Join-Path $temp 'dist'
$installDir = Join-Path $temp 'bin'
$dataDir = Join-Path $temp 'data'
$markers = @('C:\seed', '%USERPROFILE%\bin')
$registry = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)

function Read-Raw {
    $registry.GetValue(
        'Path',
        '',
        [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
    )
}

function Read-Kind {
    $registry.GetValueKind('Path')
}

function Show-State([string]$Where) {
    Write-Host "  $Where"
    Write-Host "    raw : $(Read-Raw)"
    Write-Host "    kind: $(Read-Kind)"
}

# The value as it was, so the runner is left the way it was found even when an
# assertion below fails.
$originalRaw = Read-Raw
$originalKind = $null
try {
    $originalKind = Read-Kind
} catch {
    # A runner whose user has no PATH value of their own, which is a state the
    # check has an empty-value pass for.
    $originalKind = $null
}

function Assert-Markers([string]$Where) {
    $entries = @((Read-Raw) -split ';')
    foreach ($marker in $markers) {
        if ($entries -notcontains $marker) {
            throw "${Where}: the user PATH no longer holds '$marker' as written: $(Read-Raw)"
        }
    }
    if ((Read-Kind) -ne [Microsoft.Win32.RegistryValueKind]::ExpandString) {
        throw "${Where}: the user PATH is now $((Read-Kind)), so every variable in it is frozen: $(Read-Raw)"
    }
}

try {
    New-Item -ItemType Directory -Force -Path (Join-Path $dist $package) | Out-Null
    Copy-Item -LiteralPath $Binary -Destination (Join-Path $dist "$package\leteo.exe")
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'uninstall.ps1') -Destination (Join-Path $dist "$package\uninstall.ps1")
    Compress-Archive -Path (Join-Path $dist $package) -DestinationPath (Join-Path $dist "$package.zip") -Force
    $hash = (Get-FileHash (Join-Path $dist "$package.zip") -Algorithm SHA256).Hash.ToLower()
    "$hash  $package.zip" | Set-Content -Encoding ascii (Join-Path $dist 'SHA256SUMS')

    # A free port, asked for and released before the server takes it.
    $probe = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $probe.Start()
    $port = $probe.LocalEndpoint.Port
    $probe.Stop()

    $serverScript = Join-Path $temp 'serve.ps1'
    @'
param($Root, $Port)
$listener = [System.Net.HttpListener]::new()
$listener.Prefixes.Add("http://localhost:$Port/")
$listener.Start()
while ($true) {
    $context = $listener.GetContext()
    $name = Split-Path -Leaf $context.Request.Url.AbsolutePath
    $file = Join-Path $Root $name
    if ($name -and (Test-Path -LiteralPath $file -PathType Leaf)) {
        $bytes = [System.IO.File]::ReadAllBytes($file)
        $context.Response.OutputStream.Write($bytes, 0, $bytes.Length)
    } else {
        $context.Response.StatusCode = 404
    }
    $context.Response.Close()
}
'@ | Set-Content -Encoding ascii $serverScript
    $engine = [System.Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
    $server = Start-Process -FilePath $engine -PassThru -ArgumentList @(
        '-NoProfile', '-File', "`"$serverScript`"", "`"$dist`"", "$port"
    )
    # Answered rather than slept through: a busy runner takes longer than any
    # fixed wait that would still be short enough here, and the installer's
    # failure would read as a PATH defect.
    $ready = $false
    for ($attempt = 0; $attempt -lt 50 -and -not $ready; $attempt++) {
        try {
            $response = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Uri "http://localhost:$port/SHA256SUMS"
            $ready = $response.StatusCode -eq 200
        } catch {
            Start-Sleep -Milliseconds 200
        }
    }
    if (-not $ready) {
        throw "the archive server never answered on port $port"
    }

    try {
        $seed = ((@($originalRaw) + $markers) | Where-Object { $_ }) -join ';'
        $registry.SetValue('Path', $seed, [Microsoft.Win32.RegistryValueKind]::ExpandString)
        Show-State 'seeded'
        Assert-Markers 'seeded'

        $env:LETEO_VERSION = $version
        $env:LETEO_BASE_URL = "http://localhost:$port"
        $env:LETEO_INSTALL_DIR = $installDir
        $env:LETEO_DATA_DIR = $dataDir

        & (Join-Path $PSScriptRoot 'install.ps1')
        Show-State 'after install'
        Assert-Markers 'after install'
        if (@((Read-Raw) -split ';') -notcontains $installDir) {
            throw "after install: $installDir was not added to the user PATH: $(Read-Raw)"
        }

        & (Join-Path $installDir 'uninstall.ps1') -Yes
        Show-State 'after uninstall'
        Assert-Markers 'after uninstall'
        if (@((Read-Raw) -split ';') -contains $installDir) {
            throw "after uninstall: $installDir is still on the user PATH: $(Read-Raw)"
        }
        # Not only the markers: every entry that was there is still there, in
        # the order it was in. Empty entries are the one thing the installer
        # drops, which is the `;` this check exists for.
        $seeded = @($seed -split ';' | Where-Object { $_ })
        $after = @((Read-Raw) -split ';' | Where-Object { $_ })
        if (($after -join ';') -ne ($seeded -join ';')) {
            throw "after uninstall: the PATH reads '$(Read-Raw)', not the '$(($seeded -join ';'))' it was seeded with"
        }

        # An entry that names the install directory through a variable is that
        # entry: the installer must not add a second copy of it, and the
        # uninstaller must take it away.
        $variable = $installDir -replace ('^' + [regex]::Escape($env:TEMP)), '%TEMP%'
        if ($variable -eq $installDir) {
            throw "cannot spell $installDir through %TEMP%; the variable pass needs it under $env:TEMP"
        }
        $registry.SetValue('Path', "$seed;$variable", [Microsoft.Win32.RegistryValueKind]::ExpandString)
        & (Join-Path $PSScriptRoot 'install.ps1')
        $named = @((Read-Raw) -split ';' | Where-Object {
            $_ -and [Environment]::ExpandEnvironmentVariables($_) -eq $installDir
        })
        if ($named.Count -ne 1) {
            throw "with '$variable' already there, install left $($named.Count) entries for ${installDir}: $(Read-Raw)"
        }
        & (Join-Path $installDir 'uninstall.ps1') -Yes
        $named = @((Read-Raw) -split ';' | Where-Object {
            $_ -and [Environment]::ExpandEnvironmentVariables($_) -eq $installDir
        })
        if ($named.Count -ne 0) {
            throw "uninstall left '$($named -join ';')' on the user PATH: $(Read-Raw)"
        }
        Assert-Markers 'after a variable-spelled removal'
        Write-Host 'an entry spelled with a variable is the same entry, added once and removed'

        # A user with no PATH of their own. The installer used to write
        # `"$userPath;$installDir"`, which from an empty value starts with `;` —
        # an empty entry, and Windows reads an empty entry of PATH as the
        # current directory.
        $registry.DeleteValue('Path', $false)
        & (Join-Path $PSScriptRoot 'install.ps1')
        if ((Read-Raw) -ne $installDir) {
            throw "from an empty PATH, install wrote '$(Read-Raw)' instead of just $installDir"
        }
        if ((Read-Kind) -ne [Microsoft.Win32.RegistryValueKind]::ExpandString) {
            throw "from an empty PATH, install wrote a $((Read-Kind)) value, so every variable in it would be frozen"
        }
        & (Join-Path $installDir 'uninstall.ps1') -Yes
        if (Read-Raw) {
            throw "from an empty PATH, uninstall left '$(Read-Raw)'"
        }
        Write-Host 'an empty user PATH gets Leteo alone, with no empty entry'

        Write-Host "the rest of the user PATH survived install and uninstall under $($PSVersionTable.PSVersion)"
    } finally {
        if ($server -and -not $server.HasExited) {
            Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
        }
    }
} finally {
    if ($originalKind -eq [Microsoft.Win32.RegistryValueKind]::ExpandString) {
        $registry.SetValue('Path', $originalRaw, [Microsoft.Win32.RegistryValueKind]::ExpandString)
    } elseif ($originalRaw -and $originalKind) {
        $registry.SetValue('Path', $originalRaw, $originalKind)
    } else {
        # There was no value before the check and there is no value to put back.
        $registry.DeleteValue('Path', $false)
    }
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}

# The step's exit code would otherwise be `$LASTEXITCODE`, which the last native
# command left: `leteo uninstall` exits non-zero when it keeps a file it did not
# create, and that is the uninstaller's answer about itself, not this check's.
# Reaching this line means every assertion above held.
exit 0
