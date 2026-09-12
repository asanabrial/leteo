# Fails if a Windows Leteo binary imports any DLL outside the allow-list.
#
# The release job runs this against the same `leteo.exe` it is about to archive,
# so the README's "nothing to install first" promise is measured on the artifact
# that ships rather than on a different build. The allow-list lives here and
# nowhere else: `.cargo/config.toml` points at this file instead of restating it.
#
# Delay-loaded DLLs still appear in the PE (under the delay-import directory).
# Removing `/DELAYLOAD` moves them to the ordinary import table; the set is the
# same either way, so this check does not go red when the delay loads are taken
# off — only when a DLL outside the list appears, which is what dropping
# `+crt-static` does (VCRUNTIME140.dll and the api-ms-win-crt-* forwarders).
#
# Usage: pwsh scripts/check-windows-imports.ps1 path\to\leteo.exe

$ErrorActionPreference = 'Stop'

# The DLLs a correct x86_64-pc-windows-msvc release is allowed to import.
# Lowercase, with the .dll suffix. Compared case-insensitively to what the PE
# carries. Kernel32 and ntdll are the loader's baseline; ws2_32 is eager (see
# `.cargo/config.toml`); the seven that follow are the delay-loaded set; the
# last two are what Rust's std pulls in on Windows 10+.
$AllowList = @(
    'kernel32.dll'
    'ntdll.dll'
    'ws2_32.dll'
    'crypt32.dll'
    'combase.dll'
    'shell32.dll'
    'userenv.dll'
    'user32.dll'
    'advapi32.dll'
    'bcrypt.dll'
    'api-ms-win-core-synch-l1-2-0.dll'
    'bcryptprimitives.dll'
)

if ($args.Count -ne 1) {
    Write-Error "usage: check-windows-imports.ps1 <path-to-leteo.exe>"
    exit 2
}

$path = $args[0]
if (-not (Test-Path -LiteralPath $path)) {
    Write-Error "no such file: $path"
    exit 2
}

function Get-RvaFileOffset {
    param(
        [byte[]]$Bytes,
        [uint32]$Rva,
        [int]$SectionTableOffset,
        [uint16]$NumberOfSections
    )
    for ($i = 0; $i -lt $NumberOfSections; $i++) {
        $off = $SectionTableOffset + ($i * 40)
        $virtualSize = [BitConverter]::ToUInt32($Bytes, $off + 8)
        $virtualAddress = [BitConverter]::ToUInt32($Bytes, $off + 12)
        $sizeOfRawData = [BitConverter]::ToUInt32($Bytes, $off + 16)
        $pointerToRawData = [BitConverter]::ToUInt32($Bytes, $off + 20)
        $span = [Math]::Max($virtualSize, $sizeOfRawData)
        if ($Rva -ge $virtualAddress -and $Rva -lt ($virtualAddress + $span)) {
            return [int]($pointerToRawData + ($Rva - $virtualAddress))
        }
    }
    throw "RVA 0x$($Rva.ToString('X')) is outside every section"
}

function Read-CString {
    param([byte[]]$Bytes, [int]$Offset)
    $end = $Offset
    while ($end -lt $Bytes.Length -and $Bytes[$end] -ne 0) { $end++ }
    return [System.Text.Encoding]::ASCII.GetString($Bytes, $Offset, $end - $Offset)
}

function Get-DllsFromDescriptors {
    param(
        [byte[]]$Bytes,
        [uint32]$DirectoryRva,
        [int]$SectionTableOffset,
        [uint16]$NumberOfSections,
        [int]$DescriptorSize,
        [int]$NameFieldOffset
    )
    $names = New-Object System.Collections.Generic.List[string]
    if ($DirectoryRva -eq 0) { return $names }

    $dirOffset = Get-RvaFileOffset -Bytes $Bytes -Rva $DirectoryRva `
        -SectionTableOffset $SectionTableOffset -NumberOfSections $NumberOfSections
    $index = 0
    while ($true) {
        $desc = $dirOffset + ($index * $DescriptorSize)
        $allZero = $true
        for ($b = 0; $b -lt $DescriptorSize; $b++) {
            if ($Bytes[$desc + $b] -ne 0) { $allZero = $false; break }
        }
        if ($allZero) { break }

        $nameRva = [BitConverter]::ToUInt32($Bytes, $desc + $NameFieldOffset)
        if ($nameRva -eq 0) { break }
        $nameOffset = Get-RvaFileOffset -Bytes $Bytes -Rva $nameRva `
            -SectionTableOffset $SectionTableOffset -NumberOfSections $NumberOfSections
        $names.Add((Read-CString -Bytes $Bytes -Offset $nameOffset))
        $index++
    }
    return $names
}

$bytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $path))
if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) {
    Write-Error "$path is not a PE executable (missing MZ)"
    exit 2
}

$peOffset = [BitConverter]::ToInt32($bytes, 0x3C)
if ($peOffset -lt 0 -or ($peOffset + 24) -gt $bytes.Length) {
    Write-Error "$path has a broken PE header offset"
    exit 2
}
if ([BitConverter]::ToUInt32($bytes, $peOffset) -ne 0x00004550) {
    Write-Error "$path is not a PE executable (missing PE signature)"
    exit 2
}

$numberOfSections = [BitConverter]::ToUInt16($bytes, $peOffset + 6)
$sizeOfOptionalHeader = [BitConverter]::ToUInt16($bytes, $peOffset + 20)
$optOffset = $peOffset + 24
$magic = [BitConverter]::ToUInt16($bytes, $optOffset)
if ($magic -ne 0x20B) {
    Write-Error "$path is not PE32+ (magic 0x$($magic.ToString('X'))); this check is for the x64 release binary"
    exit 2
}

# PE32+ optional header: data directories start at offset 112 from the optional header.
$dataDir = $optOffset + 112
$importRva = [BitConverter]::ToUInt32($bytes, $dataDir + (1 * 8))
$delayRva = [BitConverter]::ToUInt32($bytes, $dataDir + (13 * 8))
$sectionTableOffset = $optOffset + $sizeOfOptionalHeader

$imported = New-Object System.Collections.Generic.List[string]
foreach ($name in (Get-DllsFromDescriptors -Bytes $bytes -DirectoryRva $importRva `
            -SectionTableOffset $sectionTableOffset -NumberOfSections $numberOfSections `
            -DescriptorSize 20 -NameFieldOffset 12)) {
    $imported.Add($name)
}
foreach ($name in (Get-DllsFromDescriptors -Bytes $bytes -DirectoryRva $delayRva `
            -SectionTableOffset $sectionTableOffset -NumberOfSections $numberOfSections `
            -DescriptorSize 32 -NameFieldOffset 4)) {
    $imported.Add($name)
}

$allowed = @{}
foreach ($dll in $AllowList) { $allowed[$dll.ToLowerInvariant()] = $true }

$forbidden = New-Object System.Collections.Generic.List[string]
$seen = @{}
foreach ($dll in $imported) {
    $key = $dll.ToLowerInvariant()
    # Some toolchains write the basename only; normalise either form.
    $leaf = [System.IO.Path]::GetFileName($key)
    if ($seen.ContainsKey($leaf)) { continue }
    $seen[$leaf] = $true
    if (-not $allowed.ContainsKey($leaf)) {
        $forbidden.Add($dll)
    }
}

Write-Host "Imports checked in $path ($($seen.Count) distinct DLLs):"
foreach ($dll in ($seen.Keys | Sort-Object)) {
    $mark = if ($allowed.ContainsKey($dll)) { 'ok' } else { 'FORBIDDEN' }
    Write-Host ("  [{0}] {1}" -f $mark, $dll)
}

if ($forbidden.Count -gt 0) {
    Write-Host ""
    Write-Host "error: $path imports DLLs outside the allow-list in scripts/check-windows-imports.ps1:"
    foreach ($dll in $forbidden) {
        Write-Host "  - $dll"
    }
    Write-Host "A VCRUNTIME140.dll or api-ms-win-crt-* entry usually means +crt-static was dropped from .cargo/config.toml (or RUSTFLAGS replaced the target rustflags wholesale)."
    exit 1
}

Write-Host "All imports are on the allow-list."
exit 0
