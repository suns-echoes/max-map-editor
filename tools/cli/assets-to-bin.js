#!/usr/bin/env node

//
// ▰▰▰ CLI HELP ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰ ▰▰▰
//

import { CliLog } from '../_common/node/cli-log.js';
import fs from 'fs';
import path from 'path';

const cliLog = new CliLog();
const appDir = process.cwd();

if (process.argv[2] === '-h' || process.argv[2] === '--help') {
	cliLog.bgYellow().black()
		.ln('')
		.ln('')
		.ln(' ▟██▙  ██▙           ▟██▘      ▜█▙   ▟█▛')
		.ln('  ███▙ ███▙         ▟███▙       ▜█▙ ▟█▛')
		.ln('  ██▜█▙██▜█▙       ▟█▛ ▜█▙       🬸███🬴')
		.ln('  ██ ▜███ ▜█▙ ▜█▛ ▟███████▙ ▜█▛ ▟█▛ ▜█▙ ▜█▛')
		.ln('  ██  ▜██  ▜█▙ ▀ ▟█▛     ▜█▙ ▀ ▟█▛   ▜█▙ ▀')
		.ln('', true)
		.ln('')
		.bold().green().ln('Converts old hex string assets to binary files (v 1.0.0)', true)
		.ln('')
		.ln('Convert WRL map file into JSON format file.')
		.ln('')
		.ln('Usage:')
		.ln('    ./assets-to-bin.js input_file output_dir')
		.ln('')
		.ln('Arguments:')
		.ln('    input_file - path to the JSON file with assets encoded as hex strings')
		.ln('    output_dir - path to the directory for binary output files')
		.ln('')
		.ln('Options:')
		.ln('    -r, --relative  Resolve output_dir relative to the input file directory')
		.ln('')
		.ln('Example:')
		.ln(`    ./assets-to-bin.js ./data/base64-assets.json ./data/bin-assets`)
		.ln('')
		.ln('')
		.ln('Path:')
		.green(appDir)
		.ln('')
		.reset().log();

	process.exit(0);
}

function hexToBin(hexString) {
	const binaryString = Buffer.from(hexString, 'hex');
	return binaryString;
}

const args = process.argv.slice(2);
const positionalArgs = [];
let resolveRelativeToInput = false;

for (const arg of args) {
	if (arg === '-r' || arg === '--relative') {
		resolveRelativeToInput = true;
		continue;
	}
	if (arg.startsWith('--relative=')) {
		cliLog.red().ln('Error: --relative option does not accept a value.').reset().log();
		process.exit(1);
	}
	positionalArgs.push(arg);
}

const [inputFilePath, outputDirArg] = positionalArgs;

if (!inputFilePath || !outputDirArg) {
	cliLog.red().ln('Error: Missing required arguments.').reset().log();
	cliLog.ln('Use -h or --help for usage information.').log();
	process.exit(1);
}

const resolvedInputFile = path.isAbsolute(inputFilePath)
	? inputFilePath
	: path.resolve(appDir, inputFilePath);

let outputDir;

if (resolveRelativeToInput) {
	outputDir = path.resolve(path.dirname(resolvedInputFile), outputDirArg);
} else {
	outputDir = path.isAbsolute(outputDirArg)
 		? outputDirArg
 		: path.resolve(appDir, outputDirArg);
}

if (!fs.existsSync(resolvedInputFile) || !fs.statSync(resolvedInputFile).isFile()) {
	cliLog.red().ln(`Error: Input file "${resolvedInputFile}" does not exist or is not a file.`).reset().log();
	process.exit(1);
}

if (path.extname(resolvedInputFile).toLowerCase() !== '.json') {
	cliLog.red().ln(`Error: Input file "${resolvedInputFile}" must have a .json extension.`).reset().log();
	process.exit(1);
}

if (!fs.existsSync(outputDir)) {
	fs.mkdirSync(outputDir, { recursive: true });
	cliLog.yellow().ln(`Output directory "${outputDir}" created.`).reset().log();
}

cliLog.ln(`Processing file "${path.basename(resolvedInputFile)}"...`).log();

let tilesDataJson;

try {
	tilesDataJson = JSON.parse(fs.readFileSync(resolvedInputFile, 'utf-8'));
} catch (error) {
	cliLog.red().ln(`Error: Failed to parse JSON in file "${resolvedInputFile}": ${error.message}`).reset().log();
	process.exit(1);
}

for (const assetName of Object.keys(tilesDataJson)) {
	const base64Data = tilesDataJson[assetName];
	const binaryData = hexToBin(base64Data);

	const assetOutputFilePath = path.join(outputDir, assetName);

	fs.writeFileSync(assetOutputFilePath, binaryData);
	cliLog.green().ln(`Converted asset "${assetName}" to binary file.`).reset().log();
}

cliLog.green().ln('All done!').reset().log();
