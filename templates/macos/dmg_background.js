// Draws the install window background of an `aimer package` disk image:
// a soft gradient tinted with the colors of the app's icon, the "drag to
// Applications" instruction and three chevrons between the app icon and the
// Applications shortcut.
//
// Note: JXA runs zero-argument methods (`setFill`, `stroke`, ...) on property
// access, so they are written without parentheses.
//
// Usage: osascript -l JavaScript dmg_background.js <out.png> <app name> \
//          <width> <height> <chevron x> <chevron y> <path to the .app>
// Sizes are in points; the PNG is written at 2x so it stays sharp on Retina
// displays (Finder reads the 144 dpi and shows it at <width> x <height>).
ObjC.import('AppKit');

// The color the icon is mostly made of, as [r, g, b] in 0...1.
//
// Pixels are weighted by opacity and saturation, so the white plate or gray
// shadow most icons sit on does not wash the result out. An icon with no
// saturated pixels (a monochrome icon) yields null.
function dominantColor(appPath) {
  const side = 32;
  const rep = $.NSBitmapImageRep.alloc.initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(
    $(), side, side, 8, 4, true, false, $.NSDeviceRGBColorSpace, 0, 0);
  const context = $.NSGraphicsContext.graphicsContextWithBitmapImageRep(rep);
  $.NSGraphicsContext.setCurrentContext(context);
  const icon = $.NSWorkspace.sharedWorkspace.iconForFile(appPath);
  icon.drawInRectFromRectOperationFraction(
    { origin: { x: 0, y: 0 }, size: { width: side, height: side } },
    { origin: { x: 0, y: 0 }, size: { width: 0, height: 0 } },
    $.NSCompositingOperationSourceOver, 1);
  context.flushGraphics;

  let r = 0, g = 0, b = 0, total = 0;
  for (let y = 0; y < side; y++) {
    for (let x = 0; x < side; x++) {
      const c = rep.colorAtXY(x, y);
      const weight = c.alphaComponent * c.saturationComponent * c.saturationComponent;
      r += c.redComponent * weight;
      g += c.greenComponent * weight;
      b += c.blueComponent * weight;
      total += weight;
    }
  }
  return total < 1 ? null : [r / total, g / total, b / total];
}

function run(argv) {
  const out = argv[0];
  const appName = argv[1];
  const width = Number(argv[2]);
  const height = Number(argv[3]);
  const chevronX = Number(argv[4]);
  // Finder positions measure from the top-left; AppKit draws from the bottom-left.
  const chevronY = height - Number(argv[5]);
  const tint = dominantColor(argv[6]) || [0.45, 0.55, 0.95];
  const scale = 2;

  const rep = $.NSBitmapImageRep.alloc.initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(
    $(), width * scale, height * scale, 8, 4, true, false, $.NSDeviceRGBColorSpace, 0, 0);
  rep.size = { width: width, height: height };

  const context = $.NSGraphicsContext.graphicsContextWithBitmapImageRep(rep);
  $.NSGraphicsContext.setCurrentContext(context);

  const rgb = (r, g, b, a) => $.NSColor.colorWithSRGBRedGreenBlueAlpha(r / 255, g / 255, b / 255, a);
  const frame = { origin: { x: 0, y: 0 }, size: { width: width, height: height } };

  // Background: near-white, washed with the icon's color. Light tints of it
  // glow in from the top corners and rise from the bottom edge; the bottom one
  // is turned slightly in hue, so the wash is not one flat color.
  const tinted = (amount, hueShift, alpha) => {
    const base = $.NSColor.colorWithSRGBRedGreenBlueAlpha(tint[0], tint[1], tint[2], 1);
    const hue = (base.hueComponent + hueShift + 1) % 1;
    const saturation = Math.min(1, base.saturationComponent) * amount;
    return $.NSColor.colorWithHueSaturationBrightnessAlpha(hue, saturation, 1 - (1 - amount) * 0.04, alpha);
  };
  rgb(248, 248, 249, 1).setFill;
  $.NSBezierPath.fillRect(frame);
  const bottom = $.NSGradient.alloc.initWithStartingColorEndingColor(tinted(0.42, 0.03, 1), tinted(0.2, 0.03, 0));
  bottom.drawInRectAngle({ origin: { x: 0, y: 0 }, size: { width: width, height: height * 0.6 } }, 90);
  const top = $.NSGradient.alloc.initWithStartingColorEndingColor(tinted(0.3, 0, 0), tinted(0.3, 0, 0.9));
  top.drawInRectAngle({ origin: { x: 0, y: height * 0.7 }, size: { width: width, height: height * 0.3 } }, 90);

  // Instruction line, with the app name and "Applications" emphasized. The
  // attribute keys are the string values of NSFontAttributeName and
  // NSForegroundColorAttributeName.
  const regular = $.NSFont.systemFontOfSizeWeight(22, $.NSFontWeightRegular);
  const bold = $.NSFont.systemFontOfSizeWeight(22, $.NSFontWeightSemibold);
  const dim = rgb(110, 110, 115, 1);
  const ink = rgb(20, 20, 22, 1);
  const runs = [
    ['To install, drag ', regular, dim],
    [appName, bold, ink],
    [' to ', regular, dim],
    ['Applications', bold, ink],
  ].map(([string, font, color]) => {
    const attributes = $({ NSFont: font, NSColor: color });
    return { string: $(string), attributes: attributes, width: $(string).sizeWithAttributes(attributes).width };
  });
  let x = (width - runs.reduce((sum, run) => sum + run.width, 0)) / 2;
  for (const run of runs) {
    run.string.drawAtPointWithAttributes({ x: x, y: height - 66 }, run.attributes);
    x += run.width;
  }

  // Three chevrons, fading in toward the destination.
  const shades = [rgb(190, 190, 196, 1), rgb(120, 120, 126, 1), rgb(20, 20, 22, 1)];
  shades.forEach((color, i) => {
    const x = chevronX - 22 + i * 22;
    const path = $.NSBezierPath.bezierPath;
    path.moveToPoint({ x: x - 6, y: chevronY + 15 });
    path.lineToPoint({ x: x + 6, y: chevronY });
    path.lineToPoint({ x: x - 6, y: chevronY - 15 });
    path.lineWidth = 4;
    path.lineCapStyle = $.NSLineCapStyleRound;
    path.lineJoinStyle = $.NSLineJoinStyleRound;
    color.setStroke;
    path.stroke;
  });

  context.flushGraphics;

  const png = rep.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $());
  if (!png.writeToFileAtomically(out, true)) {
    throw new Error('could not write ' + out);
  }
  return out;
}
