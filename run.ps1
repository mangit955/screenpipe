# Runs each scenario in a fresh process with a hard 20 s timeout, takes a
# desktop screenshot at 6 s, and records the exit code.
param([string]$Exe = "target\debug\wincheck.exe", [string]$Out = "out", [string]$Only = "")
$ErrorActionPreference = 'Continue'
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$Exe = (Resolve-Path $Exe).Path
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

function Save-Desktop([string]$Path) {
  try {
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
  } catch { Write-Host "desktop screenshot failed: $_" }
}

$runs = @(
  's1 filter','s1 control','s2 filter','s2 control','s3 filter','s3 control',
  's4 filter','s4 control','s5 filter','s5 control','s6 filter','s6 control',
  's7 filter','s7 control',
  's8a filter','s8a control','s8b filter','s8b control','s8c filter','s8c control',
  's9 control','s9 filter','s9late late','s9late filter',
  's10a filter','s10a nonav','s10b filter','s10b nonav','s10c filter','s10c nonav'
)
if ($Only) { $runs = $Only.Split(',') }
$results = @()
foreach ($r in $runs) {
  $scenario, $mode = $r.Split(' ')
  $name = "$scenario-$mode"
  Write-Host "===== RUN $name ====="
  $psi = New-Object System.Diagnostics.ProcessStartInfo
  $psi.FileName = $Exe
  $psi.Arguments = "$scenario $mode `"$Out`""
  $psi.UseShellExecute = $false
  $psi.RedirectStandardOutput = $true
  $psi.RedirectStandardError = $true
  $p = [System.Diagnostics.Process]::Start($psi)
  $stdoutTask = $p.StandardOutput.ReadToEndAsync()
  $stderrTask = $p.StandardError.ReadToEndAsync()
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  while (-not $p.HasExited -and $sw.ElapsedMilliseconds -lt 6000) { Start-Sleep -Milliseconds 100 }
  if (-not $p.HasExited) { Save-Desktop (Join-Path $Out "$name-desktop.png") }
  $remaining = [Math]::Max(0, 20000 - $sw.ElapsedMilliseconds)
  if ($p.WaitForExit([int]$remaining)) {
    $p.WaitForExit()
    $code = $p.ExitCode
    $hex = '0x{0:X8}' -f $code
  } else {
    $p.Kill(); $p.WaitForExit()
    $code = 'TIMEOUT'; $hex = 'TIMEOUT'
  }
  $stdout = $stdoutTask.Result; $stderr = $stderrTask.Result
  Set-Content -Path (Join-Path $Out "$name.stdout.txt") -Value $stdout -Encoding utf8
  Set-Content -Path (Join-Path $Out "$name.stderr.txt") -Value $stderr -Encoding utf8
  Write-Host $stdout
  if ($stderr) { Write-Host "--- stderr ---"; Write-Host $stderr }
  $line = "RESULT run=$name exit=$code ($hex) elapsed_ms=$($sw.ElapsedMilliseconds)"
  Write-Host $line
  $results += $line
  Get-Process msedgewebview2 -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 500
}
Write-Host "===== ALL RESULTS ====="
$results | ForEach-Object { Write-Host $_ }
Set-Content -Path (Join-Path $Out "results.txt") -Value $results -Encoding utf8
