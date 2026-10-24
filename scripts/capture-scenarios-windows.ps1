param(
  [string]$Binary = 'D:\block-target\debug\block.exe',
  [string]$Driver = 'D:\block-tools\ui-drive3.exe',
  [string]$Contracts = 'D:\block-tools\evidence-e01-20261024\scenario-contracts.json',
  [string]$Output = 'D:\block-tools\evidence-e01-20261024\fixtures'
)
# Interactive scheduled task. These PNGs remain candidates until the local
# validator checks the native title, viewport and scenario-specific OCR text.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$utf8 = New-Object System.Text.UTF8Encoding($false)
function Write-Utf8($Path, $Text) { [IO.File]::WriteAllText($Path, $Text, $utf8) }
New-Item -ItemType Directory -Force -Path $Output | Out-Null
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class ScenarioCaptureWindow {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int left,top,right,bottom; }
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder b, int n);
}
'@
$app = $null
try {
  foreach($key in @(Get-ChildItem Env: | Where-Object { $_.Name -match '^BLOCK_(FRAME|PANE|DRAWER|TOAST)_MOTION_' } | ForEach-Object { $_.Name })) {
    Remove-Item ('Env:'+$key)
  }
  $binaryHash = (Get-FileHash $Binary -Algorithm SHA256).Hash.ToLowerInvariant()
  $buildInfo = Join-Path $Output 'build-info.json'
  Start-Process -FilePath $Binary -ArgumentList '--build-info' -RedirectStandardOutput $buildInfo -Wait
  $identity = [IO.File]::ReadAllText($buildInfo) | ConvertFrom-Json
  if(-not $identity.source_commit -or -not $identity.source_sha256) { throw 'Scenario evidence requires a source-stamped binary' }
  $cases = [IO.File]::ReadAllText($Contracts) | ConvertFrom-Json
  foreach($case in $cases) {
    foreach($theme in @('light','dark')) {
      $name = [string]$case.name
      $app = Start-Process -FilePath $Binary -ArgumentList '--scenario',$name,'--theme',$theme -PassThru
      try {
        $deadline = [DateTime]::UtcNow.AddSeconds(20)
        $settled = $false
        while([DateTime]::UtcNow -lt $deadline) {
          $app.Refresh()
          if($app.HasExited) { throw ('Scenario exited: '+$name) }
          $handle = $app.MainWindowHandle
          if($handle -ne [IntPtr]::Zero) {
            $title = New-Object Text.StringBuilder 256
            [ScenarioCaptureWindow]::GetWindowText($handle,$title,256) | Out-Null
            $rect = New-Object ScenarioCaptureWindow+Rect
            [ScenarioCaptureWindow]::GetClientRect($handle,[ref]$rect) | Out-Null
            $width = $rect.right-$rect.left
            $height = $rect.bottom-$rect.top
            if($title.ToString() -eq $case.title -and $width -eq $case.viewport[0] -and $height -eq $case.viewport[1]) {
              $settled = $true
              break
            }
          }
          Start-Sleep -Milliseconds 150
        }
        if(-not $settled) { throw ('Expected title/viewport not reached: '+$name) }
        Start-Sleep -Milliseconds 1000
        $batch = Join-Path $Output 'capture.batch'
        Write-Utf8 $batch ("shot pid="+$app.Id+" title=`""+$case.title+"`" name="+$name+'-'+$theme)
        & $Driver batch $batch $Output | Out-File (Join-Path $Output ($name+'-'+$theme+'.log')) -Encoding utf8
        if($LASTEXITCODE -ne 0) { throw ('Window capture failed: '+$name) }
        $receipt = @{
          schema_version=1; kind='scenario'; scenario=$name; platform='windows-amd64'; theme=$theme;
          source=@{commit=$identity.source_commit;source_sha256=$identity.source_sha256;dirty=$true;note='Base commit plus verified source-manifest fingerprint includes the uncommitted E03 fixture fix.'};
          binary_sha256=$binaryHash; captured_at=[DateTime]::UtcNow.ToString('o');
          window=@{title=$title.ToString();width=$width;height=$height;pid=$app.Id};
          method='Interactive scheduled task; process-owned PrintWindow client capture; local title/viewport/OCR assertions required before publication';
          data_state='Network-free disposable scenario store; synthetic Zeithrold/Nova/Pixel identities and initials; see fingerprinted scenarios.rs for exact fixture. No live sign-in or avatar claim.';
          reduced_motion=$true;focus='Scenario fixture initialization; exact native focus handle unknown';
          scroll='Fresh scenario; no scroll input; required content independently asserted';
          trigger=('--scenario '+$name+' --theme '+$theme); required_text=$case.required
        }
        Write-Utf8 (Join-Path $Output ($name+'-'+$theme+'.candidate.json')) ($receipt | ConvertTo-Json -Depth 10)
      } finally {
        if($app -and -not $app.HasExited) { $app.CloseMainWindow() | Out-Null; if(-not $app.WaitForExit(3000)) { $app.Kill() } }
        $app = $null
      }
    }
  }
  Write-Utf8 (Join-Path $Output 'done.txt') 'OK'
} catch {
  Write-Utf8 (Join-Path $Output 'done.txt') ('FAIL '+$_.Exception.Message)
  throw
}
