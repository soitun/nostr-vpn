<script lang="ts">
  import { qrMatrix, type QrMatrix } from './api';

  export let data = '';
  export let size = 186;

  let matrix: QrMatrix | null = null;
  let qrError = false;
  let generation = 0;

  $: darkPath = matrix ? matrixPath(matrix) : '';

  async function generateQr(value: string) {
    const currentGeneration = ++generation;
    matrix = null;
    qrError = false;

    if (!value) {
      return;
    }

    try {
      const next = await qrMatrix(value);
      if (currentGeneration === generation) {
        matrix = next.width > 0 ? next : null;
      }
    } catch {
      if (currentGeneration === generation) {
        qrError = true;
      }
    }
  }

  function matrixPath(next: QrMatrix): string {
    const width = next.width;
    if (width <= 0 || next.cells.length !== width * width) {
      return '';
    }

    return next.cells
      .map((dark, index) => {
        if (!dark) {
          return '';
        }
        const x = index % width;
        const y = Math.floor(index / width);
        return `M${x} ${y}h1v1h-1z`;
      })
      .join('');
  }

  $: void generateQr(data);
</script>

{#if matrix && darkPath}
  <svg
    class="qr-image"
    role="img"
    aria-label="QR code"
    width={size}
    height={size}
    viewBox={`0 0 ${matrix.width} ${matrix.width}`}
    preserveAspectRatio="none"
  >
    <rect width={matrix.width} height={matrix.width} fill="#fff" />
    <path d={darkPath} fill="#000" shape-rendering="crispEdges" />
  </svg>
{:else}
  <div class="qr-empty">{qrError ? 'QR unavailable' : 'QR'}</div>
{/if}
