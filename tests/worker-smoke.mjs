import assert from 'node:assert/strict'

import {
  cropPdfWithResult,
  getPageCount,
  WasmCropOptions,
  WasmPageBoxPolicy,
  WasmTargetAlignment,
} from '../pkg-worker/pdfcrop.js'

function createSinglePagePdf() {
  const content = '33 356 282 405 re f\n'
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources <<>> /Contents 4 0 R >>',
    `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}endstream`,
  ]

  let pdf = '%PDF-1.4\n'
  const offsets = [0]
  objects.forEach((object, index) => {
    offsets.push(Buffer.byteLength(pdf, 'binary'))
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`
  })

  const xrefOffset = Buffer.byteLength(pdf, 'binary')
  pdf += `xref\n0 ${objects.length + 1}\n`
  pdf += '0000000000 65535 f \n'
  offsets.slice(1).forEach((offset) => {
    pdf += `${String(offset).padStart(10, '0')} 00000 n \n`
  })
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\n`
  pdf += `startxref\n${xrefOffset}\n%%EOF\n`
  return new Uint8Array(Buffer.from(pdf, 'binary'))
}

const input = createSinglePagePdf()
assert.equal(getPageCount(input), 1)

const options = new WasmCropOptions()
options.setPageBoxPolicy(WasmPageBoxPolicy.Physical)
options.setTargetPage(288, 432, WasmTargetAlignment.ContentCenter)
options.setMaxProcessingMs(5_000)

const result = cropPdfWithResult(input, options)
assert.equal(getPageCount(result.pdfBytes), 1)
assert.equal(result.outputWidthPoints, 288)
assert.equal(result.outputHeightPoints, 432)
console.log('worker wasm smoke: ok')
