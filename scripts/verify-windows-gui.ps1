param(
    [Parameter(Mandatory = $true)]
    [string]$Path
)

$ErrorActionPreference = 'Stop'
$resolved = (Resolve-Path $Path).Path
$bytes = [System.IO.File]::ReadAllBytes($resolved)
if ($bytes.Length -lt 256 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) {
    throw "$resolved is not a valid PE executable"
}

$peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
if ($peOffset -lt 0 -or $peOffset + 96 -gt $bytes.Length) {
    throw "$resolved has an invalid PE header offset"
}
if ([BitConverter]::ToUInt32($bytes, $peOffset) -ne 0x00004550) {
    throw "$resolved is missing the PE signature"
}

# IMAGE_OPTIONAL_HEADER.Subsystem is at offset 68 for both PE32 and PE32+.
$optionalHeader = $peOffset + 24
$subsystem = [BitConverter]::ToUInt16($bytes, $optionalHeader + 68)
if ($subsystem -ne 2) {
    throw "$resolved uses PE subsystem $subsystem; expected 2 (Windows GUI)"
}

Write-Host "Verified Windows GUI subsystem (2): $resolved"
