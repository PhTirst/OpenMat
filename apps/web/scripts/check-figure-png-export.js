// Run with Playwright CLI against an isolated local Vite development page.
async (page) => {
  if (!/^https?:\/\/(?:localhost|127\.0\.0\.1):\d+\//.test(page.url())) {
    throw new Error('Open a local Web development page in a separate test session.');
  }
  return await page.evaluate(async () => {
    const { createFigurePngBlob } = await import('/src/components/FigureWindow.tsx');
    const { renderLatexLabel } = await import('/src/plot/latex-label.ts');
    const latexLabel = (text) => `<foreignObject x="8" y="8" width="112" height="60"><div xmlns="http://www.w3.org/1999/xhtml" class="figure-overlay-math" style="font:12px Arial;color:#000">${renderLatexLabel(text)}</div></foreignObject>`;
    const fixture = document.createElement('div');
    document.body.append(fixture);
    try {
      const results = [];
      for (const [name, label] of [
        ['plain SVG', '<text x="8" y="24" fill="#000" font-size="20">Signal</text>'],
        ['HTML math', '<foreignObject x="8" y="8" width="112" height="60"><div xmlns="http://www.w3.org/1999/xhtml" style="font:20px Arial;color:#000">x<sup>2</sup> + y<sub>1</sub></div></foreignObject>'],
        ['Unicode and hash', '<foreignObject x="8" y="8" width="112" height="60"><div xmlns="http://www.w3.org/1999/xhtml" style="font:20px Arial;color:#000">中文 #1 α</div></foreignObject>'],
        ['LaTeX prose', latexLabel('2D Function Plot')],
        ['LaTeX text and math', latexLabel('Curve $x^2$ and $y$')],
      ]) {
        fixture.innerHTML = `<canvas width="256" height="192" style="width:128px;height:96px"></canvas><svg xmlns="http://www.w3.org/2000/svg" width="128" height="96" viewBox="0 0 128 96">${label}</svg>`;
        const canvas = fixture.querySelector('canvas');
        const context = canvas.getContext('2d');
        context.fillStyle = 'rgb(20, 100, 200)';
        context.fillRect(0, 0, canvas.width, canvas.height);
        if (name === 'LaTeX prose') {
          const title = fixture.querySelector('.katex-html');
          if (title.textContent.replaceAll('\u00a0', ' ') !== '2D Function Plot') {
            throw new Error('LaTeX prose: title spaces were lost');
          }
        }
        const blob = await createFigurePngBlob(canvas, fixture.querySelector('svg'));
        const bytes = new Uint8Array(await blob.arrayBuffer());
        const signature = [137, 80, 78, 71, 13, 10, 26, 10];
        if (blob.type !== 'image/png' || !signature.every((value, index) => bytes[index] === value)) {
          throw new Error(`${name}: invalid PNG output`);
        }
        const bitmap = await createImageBitmap(blob);
        try {
          if (bitmap.width !== 256 || bitmap.height !== 192) throw new Error(`${name}: incorrect pixel size`);
          context.drawImage(bitmap, 0, 0);
          const pixel = context.getImageData(255, 191, 1, 1).data;
          if ([20, 100, 200, 255].some((value, index) => pixel[index] !== value)) {
            throw new Error(`${name}: canvas background was lost`);
          }
          const pixels = context.getImageData(0, 0, 256, 150).data;
          let labelPixels = 0;
          for (let index = 0; index < pixels.length; index += 4) {
            if (pixels[index] < 10 && pixels[index + 1] < 10 && pixels[index + 2] < 10) labelPixels += 1;
          }
          if (labelPixels < 50) throw new Error(`${name}: label content was lost`);
          results.push({ name, bytes: blob.size, width: bitmap.width, height: bitmap.height, labelPixels });
        } finally { bitmap.close(); }
      }
      return results;
    } finally { fixture.remove(); }
  });
}
