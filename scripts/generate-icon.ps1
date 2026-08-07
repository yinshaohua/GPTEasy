[CmdletBinding()]
param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot "..\src-tauri\icons")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

function New-RoundedRectanglePath {
    param(
        [System.Drawing.RectangleF]$Bounds,
        [float]$Radius
    )

    $diameter = $Radius * 2
    $path = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $path.AddArc($Bounds.Left, $Bounds.Top, $diameter, $diameter, 180, 90)
    $path.AddArc($Bounds.Right - $diameter, $Bounds.Top, $diameter, $diameter, 270, 90)
    $path.AddArc($Bounds.Right - $diameter, $Bounds.Bottom - $diameter, $diameter, $diameter, 0, 90)
    $path.AddArc($Bounds.Left, $Bounds.Bottom - $diameter, $diameter, $diameter, 90, 90)
    $path.CloseFigure()
    return $path
}

function New-IconBitmap {
    param([int]$Size)

    $bitmap = [System.Drawing.Bitmap]::new($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $graphics.Clear([System.Drawing.Color]::Transparent)

        $margin = [Math]::Max(1.0, $Size * 0.055)
        $radius = [Math]::Max(2.0, $Size * 0.18)
        $bounds = [System.Drawing.RectangleF]::new($margin, $margin, $Size - 2 * $margin, $Size - 2 * $margin)
        $backgroundPath = New-RoundedRectanglePath -Bounds $bounds -Radius $radius
        $backgroundBrush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255, 27, 31, 36))
        try {
            $graphics.FillPath($backgroundBrush, $backgroundPath)
        }
        finally {
            $backgroundBrush.Dispose()
            $backgroundPath.Dispose()
        }

        $stroke = [Math]::Max(1.5, $Size * 0.075)
        $whitePen = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(255, 245, 247, 250), $stroke)
        $greenPen = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(255, 57, 217, 139), $stroke)
        try {
            $whitePen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
            $whitePen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
            $greenPen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
            $greenPen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round

            $left = $Size * 0.25
            $right = $Size * 0.75
            $upper = $Size * 0.39
            $lower = $Size * 0.61
            $arrow = $Size * 0.12

            $graphics.DrawLine($whitePen, $left, $upper, $right, $upper)
            $graphics.DrawLine($whitePen, $right, $upper, $right - $arrow, $upper - $arrow)
            $graphics.DrawLine($whitePen, $right, $upper, $right - $arrow, $upper + $arrow)

            $graphics.DrawLine($greenPen, $right, $lower, $left, $lower)
            $graphics.DrawLine($greenPen, $left, $lower, $left + $arrow, $lower - $arrow)
            $graphics.DrawLine($greenPen, $left, $lower, $left + $arrow, $lower + $arrow)
        }
        finally {
            $whitePen.Dispose()
            $greenPen.Dispose()
        }
    }
    finally {
        $graphics.Dispose()
    }
    return $bitmap
}

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$sizes = @(16, 24, 32, 48, 64, 128, 256)
$images = [System.Collections.Generic.List[byte[]]]::new()

foreach ($size in $sizes) {
    $bitmap = New-IconBitmap -Size $size
    try {
        $stream = [System.IO.MemoryStream]::new()
        try {
            $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
            $images.Add($stream.ToArray())
        }
        finally {
            $stream.Dispose()
        }
        if ($size -eq 256) {
            $bitmap.Save((Join-Path $OutputDirectory "icon.png"), [System.Drawing.Imaging.ImageFormat]::Png)
        }
    }
    finally {
        $bitmap.Dispose()
    }
}

$iconPath = Join-Path $OutputDirectory "icon.ico"
$file = [System.IO.File]::Create($iconPath)
$writer = [System.IO.BinaryWriter]::new($file)
try {
    $writer.Write([uint16]0)
    $writer.Write([uint16]1)
    $writer.Write([uint16]$sizes.Count)
    $offset = 6 + (16 * $sizes.Count)
    for ($index = 0; $index -lt $sizes.Count; $index++) {
        $size = $sizes[$index]
        $writer.Write([byte]($(if ($size -eq 256) { 0 } else { $size })))
        $writer.Write([byte]($(if ($size -eq 256) { 0 } else { $size })))
        $writer.Write([byte]0)
        $writer.Write([byte]0)
        $writer.Write([uint16]1)
        $writer.Write([uint16]32)
        $writer.Write([uint32]$images[$index].Length)
        $writer.Write([uint32]$offset)
        $offset += $images[$index].Length
    }
    foreach ($image in $images) {
        $writer.Write($image)
    }
}
finally {
    $writer.Dispose()
    $file.Dispose()
}

Write-Output "Generated $iconPath"
