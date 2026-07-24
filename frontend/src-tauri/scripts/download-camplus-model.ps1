# 下载 CAM++ 说话人嵌入模型（3D-Speaker）用于替代 ERes2Net
$ErrorActionPreference = "Stop"
$targetDir = Join-Path $PSScriptRoot "..\sherpa-libs\models\speaker-diarization"
if (-not (Test-Path $targetDir)) { New-Item -ItemType Directory -Path $targetDir -Force | Out-Null }
$targetFile = Join-Path $targetDir "3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx"
if (Test-Path $targetFile) { Write-Host "CAM++ model already exists, skipping."; exit 0 }

$baseGithub = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx"
$mirrors = @(
    "https://gh.api.99988866.xyz/$baseGithub",
    "https://ghproxy.net/$baseGithub",
    "https://mirror.ghproxy.com/$baseGithub",
    $baseGithub
)
foreach ($url in $mirrors) {
    Write-Host "Downloading from: $url"
    try {
        Invoke-WebRequest -Uri $url -OutFile $targetFile -UseBasicParsing -TimeoutSec 60
        $size = (Get-Item $targetFile).Length
        if ($size -gt 1MB) { Write-Host "Success: $size bytes"; exit 0 }
        Remove-Item $targetFile -Force -ErrorAction SilentlyContinue
    } catch { Write-Host "Failed: $_" }
}
throw "All mirrors failed to download CAM++ model"
