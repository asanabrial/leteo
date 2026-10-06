# Removes Leteo from Windows, completely.
#
#   powershell -ExecutionPolicy Bypass -File uninstall.ps1
#
# Also what Windows runs when Leteo is uninstalled from Settings > Apps: the
# installer registers this file as the `UninstallString`, which is the only
# reason Leteo appears in that list at all. Windows provides no wizard of its
# own — it reads the registry for the list and runs whatever the entry says.
#
# # Why this is a script and not `leteo uninstall`
#
# `leteo uninstall` removes the agent configuration and the data directory, and
# it cannot remove one thing: itself. Windows holds an executable image open
# while it runs, so `leteo.exe` cannot delete `leteo.exe`. PowerShell reads a
# script into memory before executing it, so this file can delete the binary,
# the directory it sits in, and finally itself.
#
# On Linux and macOS none of this is needed — `unlink` on a running binary is
# allowed, so `leteo uninstall` finishes the job on its own.

[CmdletBinding()]
param(
    # Skip the confirmation. What the registry entry uses, because Windows may
    # run it with no console attached and a prompt nobody can see is a hang.
    [switch]$Yes,
    # Report what would go without touching anything.
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

$installDir = if ($env:LETEO_INSTALL_DIR) { $env:LETEO_INSTALL_DIR } else { "$env:LOCALAPPDATA\leteo\bin" }
$dataDir = if ($env:LETEO_DATA_DIR) { $env:LETEO_DATA_DIR } else { Join-Path $HOME '.leteo' }
$registryKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Leteo'
$binary = Join-Path $installDir 'leteo.exe'
# The model's files by name, once. tests/model_names.rs checks this line against
# MODEL_FILES in src/semantic/mod.rs, which a script cannot read.
$modelFiles = @('config.json', 'model.safetensors', 'tokenizer.json.gz')

function Say($text) { Write-Host $text }

# Said of a directory that survived. When the binary ran it already reported why
# it kept what it kept -- a hash that is not its pin, a symlink, an I/O error --
# so naming a stranger here would often be wrong. Without it, the only reason
# this script leaves a directory is that it was not empty.
function Say-Kept($directory) {
    if ($script:ran) {
        Say "  $directory was kept; the report from leteo uninstall above says why"
    } elseif (Get-ChildItem -Force $directory) {
        Say "  $directory was kept: it holds files Leteo did not put there"
    } else {
        Say "  $directory was kept: it could not be removed"
    }
}

# What is about to go, counted before anything is removed so the numbers on
# screen are the numbers that will actually be destroyed.
$memories = 'unknown'
if (Test-Path $binary) {
    try {
        $stats = & $binary stats --json 2>$null | ConvertFrom-Json
        if ($null -ne $stats.total_observations) { $memories = $stats.total_observations }
    } catch {
        # A store that cannot be read is not a reason to refuse to uninstall.
        $memories = 'unknown'
    }
}

Say "Leteo will be removed from this machine:"
Say ""
Say "  every agent it was configured in  (MCP server, hooks, memory protocol)"
Say "  $dataDir"
Say "      $memories memories, settings, and any backups kept beside them"
Say "  the semantic search model, if one was installed ($installDir\model and $dataDir\model)"
Say "  $installDir"
Say "  the PATH entry the installer added"
Say ""
# Said out loud because it is the one thing somebody might expect to go and
# must not: a `.leteo/` inside a repository is project data, usually committed,
# and frequently somebody else's as well. Hunting the filesystem for those
# would delete files out of version control.
Say "  Not touched: any .leteo/ folder inside a repository. Those are project"
Say "  files, usually committed to git and shared with the rest of a team."
Say ""

if ($DryRun) {
    Say "Nothing was removed (-DryRun)."
    exit 0
}

if (-not $Yes) {
    $answer = Read-Host "Remove all of it? This cannot be undone [y/N]"
    if ($answer -ne 'y' -and $answer -ne 'Y') {
        Say "Nothing was removed."
        exit 0
    }
}

# The agents and the data first, while the binary that knows how to find them
# still exists. It resolves fifteen agents' config files, strips the MCP server,
# the lifecycle hooks and the memory-protocol block from each, and removes the
# data directory. Doing it here by hand would be a second, worse copy of that.
# `$ran` is whether the binary started and so judged the model files, whatever it
# exited with: it exits non-zero on any incomplete removal, after it has already
# kept the model files it did not recognise. The by-name removal of the model
# below does not look at content, so it must not run behind it. Started is read
# from the line `leteo uninstall --yes` prints on stderr before it judges
# anything, the same line uninstall.sh looks for, and not from an exit code or an
# exception type: PowerShell throws when a start fails, but nothing says the
# exception was not raised after the process had begun, and an exit of 126 or 127
# is as likely to be the binary's own. `$handled` is the narrower claim that it
# finished, and gates only the data files, which the binary and this script name
# identically, so trying them again after a failure undoes no judgment.
$startedMarker = 'leteo uninstall: started'
$ran = $false
$handled = $false
if (Test-Path $binary) {
    Say "  removing agent configuration and memories"
    try {
        # Stderr is merged to read the marker, and the preference relaxed for
        # the call: Windows PowerShell 5.1 turns a native command's stderr into a
        # terminating error under 'Stop', which would end the script on the very
        # line it is looking for.
        $ErrorActionPreference = 'Continue'
        & $binary uninstall --yes 2>&1 | ForEach-Object {
            $line = "$_"
            if ($line -eq $startedMarker) { $script:ran = $true }
            Say $line
        }
        $code = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        if (-not $ran) {
            Say "  could not start leteo uninstall; carrying on with the files"
        } elseif ($code -eq 0) {
            $handled = $true
        } else {
            Say "  leteo uninstall exited with $code; carrying on with the files"
        }
    } catch {
        $ErrorActionPreference = 'Stop'
        if ($ran) {
            Say "  leteo uninstall was interrupted: $($_.Exception.Message)"
        } else {
            Say "  could not run leteo uninstall: $($_.Exception.Message)"
        }
        Say "  carrying on with the files"
    }
} else {
    Say "  no binary to ask, removing the data directory directly"
}

# Only when the binary did not finish. `leteo uninstall` removes its own files
# and leaves anything it did not create, which is the behaviour that matters and
# the one that is tested. This is the fallback for a store whose binary is gone
# or failed, so it removes the same names rather than the directory:
# `LETEO_DATA_DIR` points wherever somebody chose it to.
if ((Test-Path $dataDir) -and -not $handled) {
    Say "  removing Leteo's files from $dataDir"
    foreach ($own in @('leteo.db*', 'store.db*', 'settings.json', 'cloud.json', 'backup-*')) {
        Remove-Item -Force -Recurse (Join-Path $dataDir $own) -ErrorAction SilentlyContinue
    }
    Remove-Item -Force -Recurse (Join-Path $dataDir 'hooks') -ErrorAction SilentlyContinue
    # `leteo model install` writes here by default; left alone when the binary
    # ran, for the reason given above.
    $dataModel = Join-Path $dataDir 'model'
    if (-not $ran) {
        foreach ($own in $modelFiles) {
            Remove-Item -Force (Join-Path $dataModel $own) -ErrorAction SilentlyContinue
        }
    }
    if ((Test-Path $dataModel) -and -not (Get-ChildItem -Force $dataModel)) {
        Remove-Item -Force $dataModel -ErrorAction SilentlyContinue
    }
    if (-not (Get-ChildItem -Force $dataDir)) {
        Remove-Item -Force $dataDir -ErrorAction SilentlyContinue
    } else {
        Say-Kept $dataDir
    }
}

# The PATH entry, matched exactly rather than by substring: a user PATH holding
# `C:\tools\leteo-extras` must survive removing `C:\...\leteo\bin`.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath) {
    $kept = @($userPath -split ';' | Where-Object { $_ -and $_ -ne $installDir })
    if ($kept.Count -ne @($userPath -split ';' | Where-Object { $_ }).Count) {
        Say "  removing $installDir from your PATH"
        [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
    }
}

if (Test-Path $registryKey) {
    Say "  removing the entry from Installed apps"
    Remove-Item -Recurse -Force $registryKey -ErrorAction SilentlyContinue
}

# The two files the installer put there, by name, and then the directory only
# if that emptied it.
#
# Not `Remove-Item -Recurse` on the directory, which is what this did first and
# was a real hazard: `LETEO_INSTALL_DIR` points wherever somebody chose, so
# somebody who installed into a shared `bin` would have had every other tool in
# it deleted by uninstalling this one. A program removes what it installed, and
# nothing else — the whole complaint that produced this file.
#
# The working directory is moved out of the way first: a process cannot remove
# the directory it is sitting in, and Windows reports that as a permission error
# rather than as what it is.
Set-Location $HOME
foreach ($own in @('leteo.exe', 'uninstall.ps1')) {
    $path = Join-Path $installDir $own
    if (Test-Path $path) {
        Say "  removing $path"
        Remove-Item -Force $path -ErrorAction SilentlyContinue
    }
}
# The model the installer put beside the binary, by name, unless the binary ran
# and so already kept what it did not recognise. The directory goes only if that
# left it empty: Remove-Item prompts on a directory with anything in it, and a
# prompt hangs an uninstall that nobody is watching.
$modelDir = Join-Path $installDir 'model'
if (-not $ran) {
    foreach ($own in $modelFiles) {
        Remove-Item -Force (Join-Path $modelDir $own) -ErrorAction SilentlyContinue
    }
}
if ((Test-Path $modelDir) -and -not (Get-ChildItem -Force $modelDir)) {
    Remove-Item -Force $modelDir -ErrorAction SilentlyContinue
} elseif (Test-Path $modelDir) {
    Say-Kept $modelDir
}
if ((Test-Path $installDir) -and -not (Get-ChildItem -Force $installDir)) {
    Remove-Item -Force $installDir -ErrorAction SilentlyContinue
} elseif (Test-Path $installDir) {
    Say "  $installDir was kept: it holds files Leteo did not put there"
}

Say ""
Say "Leteo is gone. Open a new terminal for the PATH change to take effect."
