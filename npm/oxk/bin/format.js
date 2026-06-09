const { formatFiles: nativeFormatFiles } = require('../format.js')

async function formatFiles(args) {
  const success = await nativeFormatFiles(args)
  if (!success) {
    process.exitCode = 1
  }
}

module.exports = { formatFiles }
