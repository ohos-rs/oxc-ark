const { readFileSync, writeFileSync } = require('fs')
const { relative } = require('path')
const { format } = require('../format.js')
const { glob } = require('glob')

async function formatFiles(files) {
  if (files.length === 0) {
    console.error('Error: No files specified for formatting')
    process.exit(1)
  }

  let hasErrors = false
  let formattedCount = 0
  const allFiles = new Set() // Use Set to avoid duplicates

  // Use glob to expand patterns
  for (const filePattern of files) {
    try {
      const matchedFiles = await glob(filePattern, {
        absolute: true,
        ignore: ['**/node_modules/**'],
      })
      matchedFiles.forEach((file) => allFiles.add(file))
    } catch (error) {
      console.error(`Error expanding pattern "${filePattern}":`, error.message)
      hasErrors = true
    }
  }

  if (allFiles.size === 0) {
    console.error('Error: No files found matching the specified patterns')
    process.exit(1)
  }

  for (const filePath of allFiles) {
    try {
      const sourceText = readFileSync(filePath, 'utf-8')
      // Use relative path for format function (it expects the original file name)
      const relativePath = relative(process.cwd(), filePath)

      const result = await format(relativePath, sourceText)

      if (result.errors.length > 0) {
        console.error(`Error formatting ${relativePath}:`)
        result.errors.forEach((err) => console.error(`  ${err}`))
        hasErrors = true
      } else {
        writeFileSync(filePath, result.code, 'utf-8')
        console.log(`Formatted: ${relativePath}`)
        formattedCount++
      }
    } catch (error) {
      console.error(`Error processing ${filePath}:`, error.message)
      hasErrors = true
    }
  }

  console.log(`\nFormatted ${formattedCount} file(s)`)
  if (hasErrors) {
    process.exit(1)
  }
}

module.exports = { formatFiles }
