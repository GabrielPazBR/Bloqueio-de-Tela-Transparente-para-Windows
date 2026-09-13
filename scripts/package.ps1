#requires -Version 7.0
[CmdletBinding()]
param(
    [ValidateSet('Native', 'Wsl')][string]$BuildBackend = 'Wsl',
    [ValidateSet('x64', 'x86')][string]$Architecture = 'x64',
    [string]$CertificateThumbprint,
    [switch]$MachineCertificateStore,
    [string]$SigningDlib,
    [string]$SigningMetadata,
    [string]$SignTool,
    [string]$TimestampUrl = 'http://timestamp.acs.microsoft.com',
    [switch]$UnsignedDevelopment
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path $PSScriptRoot -Parent
$manifest = Get-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Raw
$version = [regex]::Match($manifest, '(?m)^version = "([^"]+)"').Groups[1].Value
if (-not $version) { throw 'Versão do aplicativo não encontrada.' }
$installerManifest = Get-Content -LiteralPath (Join-Path $repo 'installer/Cargo.toml') -Raw
if (-not $installerManifest.Contains("version = `"$version`"")) { throw 'As versões do instalador e aplicativo são diferentes.' }

# Fail before compiling or creating a distribution directory.
if ($UnsignedDevelopment) {
    if ($CertificateThumbprint -or $SigningDlib -or $SigningMetadata) {
        throw 'Não combine pacote de desenvolvimento e assinatura.'
    }
} else {
    if (-not $CertificateThumbprint -and -not ($SigningDlib -and $SigningMetadata)) {
        throw 'Assinatura obrigatória: informe CertificateThumbprint ou SigningDlib e SigningMetadata. Não há geração automática de certificado.'
    }
    if ($CertificateThumbprint -and ($SigningDlib -or $SigningMetadata)) { throw 'Escolha apenas um método de assinatura.' }
    if (-not $SignTool) {
        $SignTool = Get-ChildItem -LiteralPath "${env:ProgramFiles(x86)}/Windows Kits/10/bin" -Filter signtool.exe -Recurse |
            Where-Object { $_.Directory.Name -eq 'x64' } | Sort-Object FullName -Descending |
            Select-Object -First 1 -ExpandProperty FullName
    }
    if (-not $SignTool -or -not (Test-Path -LiteralPath $SignTool)) { throw 'SignTool não encontrado.' }
    if ($CertificateThumbprint) {
        $CertificateThumbprint = $CertificateThumbprint.Replace(' ', '')
        $store = if ($MachineCertificateStore) { 'LocalMachine' } else { 'CurrentUser' }
        $certificate = Get-Item -LiteralPath "Cert:/$store/My/$CertificateThumbprint"
        if (-not $certificate.HasPrivateKey -or $certificate.NotAfter -le (Get-Date)) { throw 'Certificado sem chave privada ou expirado.' }
        if ($certificate.EnhancedKeyUsageList.ObjectId -notcontains '1.3.6.1.5.5.7.3.3') { throw 'O certificado não permite assinatura de código.' }
    } else {
        if (-not (Test-Path -LiteralPath $SigningDlib) -or -not (Test-Path -LiteralPath $SigningMetadata)) { throw 'Arquivos do serviço de assinatura não encontrados.' }
    }
}

function Invoke-Checked([string]$Program, [string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program terminou com código $LASTEXITCODE." }
}

function Convert-WslPath([string]$Path) {
    $converted = & wsl.exe -- wslpath -a -u $Path
    if ($LASTEXITCODE -ne 0) { throw 'Não foi possível converter o caminho para WSL.' }
    return $converted.Trim()
}

function Sign-And-Verify([string]$Path) {
    if ($UnsignedDevelopment) { return }
    $arguments = @('sign', '/fd', 'SHA256', '/tr', $TimestampUrl, '/td', 'SHA256')
    if ($CertificateThumbprint) {
        $arguments += @('/s', 'My', '/sha1', $CertificateThumbprint)
        if ($MachineCertificateStore) { $arguments += '/sm' }
    } else {
        $arguments += @('/dlib', $SigningDlib, '/dmdf', $SigningMetadata)
    }
    Invoke-Checked $SignTool ($arguments + @($Path))
    Invoke-Checked $SignTool @('verify', '/pa', '/all', '/v', $Path)
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid' -or -not $signature.TimeStamperCertificate) {
        throw "Assinatura ou carimbo de tempo inválido em $Path."
    }
}

$kind = if ($UnsignedDevelopment) { 'unsigned-development' } else { 'signed' }
$release = Join-Path $repo "dist/$version/$Architecture/$kind"
if (-not $UnsignedDevelopment -and (Test-Path -LiteralPath $release)) {
    throw 'Já existe um pacote assinado para esta versão. Gere uma nova versão.'
}
$destination = Join-Path $repo "work/package-$([guid]::NewGuid())"
$null = New-Item -ItemType Directory -Path $destination -Force
$app = Join-Path $destination 'BloqueioTransparente.exe'
$setup = Join-Path $destination "BloqueioTransparente-Setup-$version-$Architecture.exe"
$target = if ($Architecture -eq 'x64') { 'x86_64-pc-windows-msvc' } else { 'i686-pc-windows-msvc' }
$metadataPath = Join-Path $destination 'package.json'
# A previous manifest must not survive a failed build/signing attempt.
if (Test-Path -LiteralPath $metadataPath) { Remove-Item -LiteralPath $metadataPath }
Push-Location $repo
$previousTarget = $env:CARGO_TARGET_DIR
if ($BuildBackend -eq 'Native') { $env:CARGO_TARGET_DIR = Join-Path $repo 'target' }
try {
    if ($BuildBackend -eq 'Wsl') {
        $script = Convert-WslPath (Join-Path $PSScriptRoot 'build-windows.sh')
        Invoke-Checked 'wsl.exe' @('--', 'bash', $script, 'app', $target, (Convert-WslPath $app))
    } else {
        Invoke-Checked 'cargo' @('build', '--locked', '--release', '--target', $target, '--bin', 'BloqueioTransparente')
        Copy-Item -LiteralPath (Join-Path $repo "target/$target/release/BloqueioTransparente.exe") -Destination $app
    }
    if ([System.Diagnostics.FileVersionInfo]::GetVersionInfo($app).FileVersion -ne $version) {
        throw 'A versão do aplicativo compilado não corresponde ao pacote.'
    }
    Sign-And-Verify $app
    $payloadHash = (Get-FileHash -LiteralPath $app -Algorithm SHA256).Hash
    if ($BuildBackend -eq 'Wsl') {
        Invoke-Checked 'wsl.exe' @('--', 'bash', $script, 'installer', $target, (Convert-WslPath $app), (Convert-WslPath $setup))
    } else {
        $previousPayload = $env:BT_APP_PAYLOAD
        try {
            $env:BT_APP_PAYLOAD = $app
            Invoke-Checked 'cargo' @('build', '--locked', '--release', '--target', $target, '-p', 'bloqueio-transparente-installer')
            Copy-Item -LiteralPath (Join-Path $repo "target/$target/release/BloqueioTransparente-Setup.exe") -Destination $setup
        } finally { $env:BT_APP_PAYLOAD = $previousPayload }
    }
    if ([System.Diagnostics.FileVersionInfo]::GetVersionInfo($setup).FileVersion -ne $version) {
        throw 'A versão do instalador compilado não corresponde ao pacote.'
    }
    if ((Get-FileHash -LiteralPath $app -Algorithm SHA256).Hash -ne $payloadHash) { throw 'O aplicativo mudou durante o empacotamento.' }
    Sign-And-Verify $setup
    $setupHash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash
    [ordered]@{
        version = $version; architecture = $Architecture; signing = $kind
        applicationSha256 = $payloadHash; installerSha256 = $setupHash
        installer = [IO.Path]::GetFileName($setup)
        runtimeValidation = 'pending-windows-install-update-repair'
    } | ConvertTo-Json | Set-Content -LiteralPath $metadataPath -Encoding utf8
    try {
        if ($BuildBackend -eq 'Wsl') {
            Invoke-Checked 'wsl.exe' @('--', 'python3', (Convert-WslPath (Join-Path $PSScriptRoot 'verify-package.py')), (Convert-WslPath $destination))
        } else {
            Invoke-Checked 'py.exe' @('-3', (Join-Path $PSScriptRoot 'verify-package.py'), $destination)
        }
    } catch {
        Remove-Item -LiteralPath $metadataPath
        throw
    }
    "$setupHash  $([IO.Path]::GetFileName($setup))" | Set-Content -LiteralPath (Join-Path $destination 'SHA256SUMS.txt') -Encoding ascii
    $previousDevelopment = Join-Path $repo "work/previous-package-$([guid]::NewGuid())"
    foreach ($path in @($destination, $release, $previousDevelopment)) {
        if (-not [IO.Path]::GetFullPath($path).StartsWith([IO.Path]::GetFullPath($repo) + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Destino do pacote fora do projeto.'
        }
    }
    $null = New-Item -ItemType Directory -Path (Split-Path $release -Parent) -Force
    if (Test-Path -LiteralPath $release) {
        Move-Item -LiteralPath $release -Destination $previousDevelopment
    }
    Move-Item -LiteralPath $destination -Destination $release
    Write-Output "Pacote preparado: $(Join-Path $release ([IO.Path]::GetFileName($setup))) ($kind)"
} finally {
    $env:CARGO_TARGET_DIR = $previousTarget
    Pop-Location
}
