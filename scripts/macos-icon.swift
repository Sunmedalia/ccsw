import AppKit
// Code-drawn vector mark: three translucent model layers on a pale glass tile.
let destination = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: true)
for size in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let pixels = size * scale
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
        let transform = AffineTransform(scale: Double(pixels) / 1024)
        (transform as NSAffineTransform).concat()
        let background = NSBezierPath(roundedRect: NSRect(x: 70, y: 70, width: 884, height: 884), xRadius: 202, yRadius: 202)
        NSGradient(starting: NSColor(red: 0.94, green: 0.97, blue: 0.98, alpha: 1), ending: NSColor(red: 0.77, green: 0.86, blue: 0.91, alpha: 1))!.draw(in: background, angle: -70)
        for (offset, alpha) in [(CGFloat(-130), CGFloat(0.50)), (CGFloat(0), CGFloat(0.72)), (CGFloat(130), CGFloat(1))] {
            let diamond = NSBezierPath()
            diamond.move(to: NSPoint(x: 235, y: 510 + offset))
            diamond.line(to: NSPoint(x: 512, y: 360 + offset))
            diamond.line(to: NSPoint(x: 789, y: 510 + offset))
            diamond.line(to: NSPoint(x: 512, y: 660 + offset))
            diamond.close()
            NSColor(red: 0.27, green: 0.57, blue: 0.65, alpha: alpha).setFill()
            diamond.fill()
            NSColor.white.withAlphaComponent(0.8).setStroke()
            diamond.lineWidth = 12
            diamond.lineJoinStyle = .round
            diamond.stroke()
        }
        NSGraphicsContext.restoreGraphicsState()
        let suffix = scale == 2 ? "@2x" : ""
        try bitmap.representation(using: .png, properties: [:])!.write(to: destination.appendingPathComponent("icon_\(size)x\(size)\(suffix).png"))
    }
}
