// Run with Playwright CLI on a local OpenMat page after running wave_planet.m.
// Uses the real pointer path, WASM renderer, GPU canvas, and kernel camera commit.
async (page) => {
  if (!/^https?:\/\/(?:localhost|127\.0\.0\.1):\d+\//.test(page.url())) {
    throw new Error('Use a separate local OpenMat test session with Wave Planet open.');
  }
  const canvas = page.getByRole('img', { name: /WebGPU rendering surface/ }).last();
  const figure = page.getByRole('figure').filter({ has: canvas });
  const dialog = page.getByRole('dialog').filter({ has: canvas });
  await dialog.getByRole('button', { name: 'Rotate 3D view', exact: true }).click();
  const bounds = await canvas.boundingBox();
  if (!bounds || bounds.width < 300 || bounds.height < 200) {
    throw new Error('A visible 3D Figure is required.');
  }
  const compare = async (first, second) => page.evaluate(async ([a, b]) => {
    const pixels = async (base64) => {
      const bytes = Uint8Array.from(atob(base64), character => character.charCodeAt(0));
      const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
      try {
        const buffer = new OffscreenCanvas(bitmap.width, bitmap.height);
        const context = buffer.getContext('2d');
        context.drawImage(bitmap, 0, 0);
        return context.getImageData(0, 0, buffer.width, buffer.height);
      } finally { bitmap.close(); }
    };
    const left = await pixels(a);
    const right = await pixels(b);
    if (left.width !== right.width || left.height !== right.height) {
      throw new Error('Figure resized during the lighting check.');
    }
    let changed = 0;
    let totalDifference = 0;
    for (let i = 0; i < left.data.length; i += 4) {
      const delta = [0, 1, 2].map(channel => Math.abs(left.data[i + channel] - right.data[i + channel]));
      if (Math.max(...delta) > 12) changed += 1;
      totalDifference += delta[0] + delta[1] + delta[2];
    }
    const count = left.width * left.height;
    return { changedFraction: changed / count, meanChannelDifference: totalDifference / (3 * count) };
  }, [first.toString('base64'), second.toString('base64')]);

  const results = [];
  for (const [index, [dx, dy]] of [[140, -35], [-170, 60]].entries()) {
    const before = await canvas.screenshot();
    const revision = await figure.locator('figcaption').textContent();
    const x = bounds.x + bounds.width * 0.5;
    const y = bounds.y + bounds.height * 0.5;
    await page.mouse.move(x, y);
    await page.mouse.down();
    let held;
    try {
      await page.mouse.move(x + dx, y + dy, { steps: 16 });
      held = await canvas.screenshot({ path: `output/playwright/orbit-lighting-${index}-held.png` });
      if (await figure.locator('figcaption').textContent() !== revision) {
        throw new Error('Orbit preview unexpectedly committed to the kernel while the button was held.');
      }
    } finally { await page.mouse.up(); }
    // Wait for the authoritative scene revision, not an arbitrary idle timeout.
    await figure.evaluate((element, previous) => new Promise((resolve, reject) => {
      const deadline = performance.now() + 10000;
      const check = () => {
        if (element.querySelector('figcaption')?.textContent !== previous) resolve();
        else if (performance.now() > deadline) reject(new Error('Camera commit did not arrive.'));
        else requestAnimationFrame(check);
      };
      check();
    }), revision);
    const committed = await canvas.screenshot({ path: `output/playwright/orbit-lighting-${index}-committed.png` });
    const movement = await compare(before, held);
    const jump = await compare(held, committed);
    if (movement.changedFraction < 0.01) {
      throw new Error(`Orbit did not visibly update the Figure: ${JSON.stringify(movement)}`);
    }
    if (jump.changedFraction > 0.005 || jump.meanChannelDifference > 0.5) {
      throw new Error(`Lighting changed after mouse release: ${JSON.stringify(jump)}`);
    }
    results.push({ drag: index + 1, movement, jump });
  }
  return results;
}
