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
# (`shell: powershell`) and once under PowerShell 7 (`shell: pwsh`). Not because
# the engines differ — measured, both read the value expanded and store it as
# plain text — but because whichever engine a user happens to run is the one
# that has to be right, and only a run under it says so.
#
# Needs a built `leteo.exe` (the checkout's `target\debug` by default): the
# cargo step before it in `ci.yml` makes one, and the script assembles the
# release archive around that binary and serves it from loopback, so it
# exercises the real `install.ps1` rather than a copy of its PATH logic.
#
# What it cannot see is whether a running program received the
# `WM_SETTINGCHANGE` the scripts broadcast after writing: that needs a
# top-level window and a message loop in this process. The value and its kind
# in the registry are what this holds.

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
$originalExists = $null -ne $registry.GetValue(
    'Path',
    $null,
    [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
)
$originalRaw = if ($originalExists) { Read-Raw } else { '' }
$originalKind = if ($originalExists) { Read-Kind } else { $null }

# Runs a script and holds its outcome. `$?` is read on the line after the
# invocation, inside the same scope: read from a separate function it is the
# status of whatever that function ran, not of the call before it.
#
# The arguments are a hashtable, not an array, because an array splatted here is
# positional: a lone `-Yes` reached `uninstall.ps1` as a value, not as a switch,
# and the whole uninstaller pass never ran. Only the windows leg could say so —
# this script needs a registry to start.
function Invoke-Script([string]$Path, [string]$What, [hashtable]$Parameters = @{}) {
    & $Path @Parameters
    if (-not $?) {
        throw "$What did not succeed (exit code $LASTEXITCODE)"
    }
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
    # A port is asked for, released, and taken again by another process, so a
    # server that does not answer is retried on a fresh one rather than being
    # reported as a PATH failure. Answered rather than slept through, for the
    # same reason: a busy runner takes longer than any fixed wait short enough
    # to be useful here.
    $server = $null
    $port = $null
    $rounds = 3
    $attempts = 50
    $waitMs = 200
    for ($round = 0; $round -lt $rounds -and -not $server; $round++) {
        $probe = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
        $probe.Start()
        $candidatePort = $probe.LocalEndpoint.Port
        $probe.Stop()
        $launched = Start-Process -FilePath $engine -PassThru -ArgumentList @(
            '-NoProfile', '-File', "`"$serverScript`"", "`"$dist`"", "$candidatePort"
        )
        for ($attempt = 0; $attempt -lt $attempts; $attempt++) {
            if ($launched.HasExited) {
                break
            }
            try {
                $response = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 -Uri "http://localhost:$candidatePort/SHA256SUMS"
                if ($response.StatusCode -eq 200) {
                    $server = $launched
                    $port = $candidatePort
                    break
                }
            } catch {
                # A refused connection is the usual answer while the server
                # starts; the wait below is what makes another attempt worth it.
            }
            # Outside the catch: a response that is not 200 is not an answer
            # either, so the wait belongs between attempts whichever way one
            # fails. Inside the catch, fifty of these could pass with no wait.
            Start-Sleep -Milliseconds $waitMs
        }
        if (-not $server -and -not $launched.HasExited) {
            Stop-Process -Id $launched.Id -Force -ErrorAction SilentlyContinue
        }
    }
    if (-not $server) {
        throw "the archive server never answered on $rounds ports"
    }

    try {
        # The runner's own value, empty entries and all, so the comparisons
        # below hold what the scripts promise: everything they did not write
        # comes back byte for byte. The `;;` is always seeded, because an empty
        # entry is what a filtering write drops and a runner that happens to have
        # one is not a test of that; so is the `;` it ends with, because a value
        # ending in a separator is a shape install has to extend without
        # inventing an entry and uninstall has to hand back unchanged.
        $seed = "$originalRaw;;" + ($markers -join ';') + ';'
        $registry.SetValue('Path', $seed, [Microsoft.Win32.RegistryValueKind]::ExpandString)
        Show-State 'seeded'
        Assert-Markers 'seeded'

        $env:LETEO_VERSION = $version
        $env:LETEO_BASE_URL = "http://localhost:$port"
        $env:LETEO_INSTALL_DIR = $installDir
        $env:LETEO_DATA_DIR = $dataDir

        Invoke-Script (Join-Path $PSScriptRoot 'install.ps1') 'install.ps1'
        Show-State 'after install'
        Assert-Markers 'after install'
        if (@((Read-Raw) -split ';') -notcontains $installDir) {
            throw "after install: $installDir was not added to the user PATH: $(Read-Raw)"
        }

        Invoke-Script (Join-Path $installDir 'uninstall.ps1') 'uninstall.ps1' @{ Yes = $true }
        Show-State 'after uninstall'
        Assert-Markers 'after uninstall'
        if (@((Read-Raw) -split ';') -contains $installDir) {
            throw "after uninstall: $installDir is still on the user PATH: $(Read-Raw)"
        }
        # Not only the markers: the value is the one it was seeded with, byte
        # for byte, so an empty entry the runner had is still an empty entry.
        if ((Read-Raw) -cne $seed) {
            throw "after uninstall: the PATH reads '$(Read-Raw)', not the '$seed' it was seeded with"
        }

        # An entry that names the install directory through a variable, and
        # with a trailing separator, is that entry: the installer must not add a
        # second copy of it, and the uninstaller must take it away.
        #
        # The variable is found by expanding it back rather than by trusting the
        # spelling: `$env:TEMP` on the runners is the 8.3 form of the directory
        # `GetTempPath` returns, so a prefix compare against it misses.
        $variable = $null
        foreach ($name in 'TEMP', 'LOCALAPPDATA', 'USERPROFILE') {
            $prefix = [Environment]::GetEnvironmentVariable($name)
            if (-not $prefix -or -not $installDir.StartsWith("$prefix\", [StringComparison]::OrdinalIgnoreCase)) {
                continue
            }
            $spelling = "%$name%" + $installDir.Substring($prefix.Length) + '\'
            if ([Environment]::ExpandEnvironmentVariables($spelling).TrimEnd('\', '/') -eq $installDir.TrimEnd('\', '/')) {
                $variable = $spelling
                break
            }
        }
        if (-not $variable) {
            throw "cannot spell $installDir through a variable; the variable pass needs it under TEMP, LOCALAPPDATA or USERPROFILE"
        }
        $literal = $installDir + '\'
        $slashed = $installDir.Replace('\', '/')
        $seededWithSpellings = "$seed;$variable;$literal;$slashed"
        $registry.SetValue('Path', $seededWithSpellings, [Microsoft.Win32.RegistryValueKind]::ExpandString)
        Invoke-Script (Join-Path $PSScriptRoot 'install.ps1') 'install.ps1 with the install directory already there'
        # The whole value, not a count of entries: install must add nothing when
        # the directory is already there in any spelling, and comparing the
        # bytes says so without the check restating how an entry folds.
        if ((Read-Raw) -cne $seededWithSpellings) {
            throw "with three spellings of ${installDir} already there, install rewrote the PATH: $(Read-Raw)"
        }
        Invoke-Script (Join-Path $installDir 'uninstall.ps1') 'uninstall.ps1 with the install directory spelled three ways' @{ Yes = $true }
        if ((Read-Raw) -cne $seed) {
            throw "after removing three spellings of ${installDir}, the PATH reads '$(Read-Raw)', not '$seed'"
        }
        Assert-Markers 'after a variable-spelled removal'
        Write-Host 'an entry spelled with a variable is the same entry, added once and removed'

        # A value the user keeps as plain text stays plain text: the scripts
        # promise the kind they found, not an expandable value every time.
        $registry.SetValue('Path', $seed, [Microsoft.Win32.RegistryValueKind]::String)
        Invoke-Script (Join-Path $PSScriptRoot 'install.ps1') 'install.ps1 on a plain-text PATH'
        if ((Read-Kind) -ne [Microsoft.Win32.RegistryValueKind]::String) {
            throw "install turned a plain-text PATH into a $((Read-Kind)) one: $(Read-Raw)"
        }
        Invoke-Script (Join-Path $installDir 'uninstall.ps1') 'uninstall.ps1 on a plain-text PATH' @{ Yes = $true }
        if ((Read-Kind) -ne [Microsoft.Win32.RegistryValueKind]::String) {
            throw "uninstall turned a plain-text PATH into a $((Read-Kind)) one: $(Read-Raw)"
        }
        if ((Read-Raw) -cne $seed) {
            throw "after a plain-text round trip, the PATH reads '$(Read-Raw)', not '$seed'"
        }
        Write-Host 'a plain-text PATH stays plain text'

        # A user with no PATH of their own. The installer used to write
        # `"$userPath;$installDir"`, which from an empty value starts with `;` —
        # an empty entry, and Windows reads an empty entry of PATH as the
        # current directory.
        $registry.DeleteValue('Path', $false)
        Invoke-Script (Join-Path $PSScriptRoot 'install.ps1') 'install.ps1 from an empty PATH'
        if ((Read-Raw) -ne $installDir) {
            throw "from an empty PATH, install wrote '$(Read-Raw)' instead of just $installDir"
        }
        if ((Read-Kind) -ne [Microsoft.Win32.RegistryValueKind]::ExpandString) {
            throw "from an empty PATH, install wrote a $((Read-Kind)) value, so every variable in it would be frozen"
        }
        Invoke-Script (Join-Path $installDir 'uninstall.ps1') 'uninstall.ps1 from an empty PATH' @{ Yes = $true }
        if ($null -ne $registry.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)) {
            throw "from an empty PATH, uninstall left the value '$(Read-Raw)' where the environment API deletes it"
        }
        Write-Host 'an empty user PATH gets Leteo alone, with no empty entry'

        Write-Host "the rest of the user PATH survived install and uninstall under $($PSVersionTable.PSVersion)"
    } finally {
        if ($server -and -not $server.HasExited) {
            Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
        }
    }
} finally {
    if ($originalExists) {
        # Back as it was, an empty value included: the check must not turn a
        # value somebody had into an absent one.
        $registry.SetValue('Path', $originalRaw, $originalKind)
    } else {
        $registry.DeleteValue('Path', $false)
    }
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}

# The step's exit code would otherwise be `$LASTEXITCODE`, which the last native
# command left: `leteo uninstall` exits non-zero when it keeps a file it did not
# create, and that is the uninstaller's answer about itself, not this check's.
# Reaching this line means every assertion above held.
exit 0
