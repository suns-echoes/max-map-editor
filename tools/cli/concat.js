#!/bin/env node

/**
 * INSTRUCTIONs:
 * 
 * 1. Place this script in the directory containing the `tiles.data` folder with binary tile files.
 * 2. Run the script.
 * 3. The script will create `tiles-data.bin` and `tiles-data.bin.json` in the same directory.
 */

/**
 * Concatenate all files in ./tiles-data dir into a single binary file ./tiles-data.bin.
 * Also creates the corresponding ./tiles-data.bin.json metadata file withe the list of file names in order of concatenation.
 */
import { readdirSync, statSync, createWriteStream, readFileSync, writeFileSync } from 'fs';
import { join } from 'path';

const __dirname = process.cwd();

const inputDir = join(__dirname, 'tiles.data');
const outputFile = join(__dirname, 'tiles-data.bin');
const outputMetaFile = join(__dirname, 'tiles-data.bin.json');

async function concatFiles() {
    const files = readdirSync(inputDir).filter(file => statSync(join(inputDir, file)).isFile());
    const writeStream = createWriteStream(outputFile);
    const metadata = [];

    for (const file of files) {
        const filePath = join(inputDir, file);
        const data = readFileSync(filePath);
        writeStream.write(data);
        metadata.push(file);
    }

    writeStream.end();

    writeFileSync(outputMetaFile, JSON.stringify(metadata, null, 2));
    console.log(`Concatenated ${files.length} files into ${outputFile}`);
    console.log(`Metadata written to ${outputMetaFile}`);
}

concatFiles().catch(err => {
    console.error('Error during concatenation:', err);
});