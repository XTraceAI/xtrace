import assert from 'node:assert/strict';
import { test as it } from 'node:test';
import { resolve, join } from 'node:path';
import { readFile, readdir } from 'node:fs/promises';
import ts from 'typescript';
import { ESLint } from 'eslint';
const ui = resolve(import.meta.dirname, '..');
const repo = resolve(ui, '../../..');

it('rejects re-exports, dynamic imports, require and globs including unguarded entry imports', async () => {
  const lint = new ESLint({ cwd: repo });
  for (const source of [
    "import { sample } from '../gallery/sample'; void sample;",
    "export * from '../gallery/sample';",
    "void import('../gallery/sample');",
    "void require('../gallery/' + 'sample');",
    "void import.meta.glob('../gallery/*.ts');",
    'void import(`../gallery/sample`);',
    'void import(`../gallery/${name}.tsx`);',
    "void import.meta.glob('../**/*.tsx');",
  ]) {
    const [result] = await lint.lintText(source, { filePath: join(ui, 'src/app/negative.ts') });
    assert.ok(
      result.messages.some(({ ruleId }) => ruleId === 'gallery/boundary'),
      source,
    );
  }
  const [entry] = await lint.lintText("void import('../gallery/Gallery');", {
    filePath: join(ui, 'src/app/AppRoutes.tsx'),
  });
  assert.ok(entry.messages.some(({ ruleId }) => ruleId === 'gallery/boundary'));
  const [real] = await lint.lintFiles([join(ui, 'src/app/AppRoutes.tsx')]);
  assert.equal(real.errorCount, 0);
});

it('exports every kit component through the public barrel', async () => {
  const components = [];
  for (const file of await readdir(join(ui, 'src/kit'))) {
    if (!file.endsWith('.tsx') || file.includes('.test.')) continue;
    const source = ts.createSourceFile(
      file,
      await readFile(join(ui, 'src/kit', file), 'utf8'),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
    for (const node of source.statements) {
      if (
        ts.isFunctionDeclaration(node) &&
        node.modifiers?.some((value) => value.kind === ts.SyntaxKind.ExportKeyword) &&
        node.name &&
        /^[A-Z]/.test(node.name.text)
      )
        components.push(node.name.text);
    }
  }
  const barrel = ts.createSourceFile(
    'index.ts',
    await readFile(join(ui, 'src/kit/index.ts'), 'utf8'),
    ts.ScriptTarget.Latest,
    true,
  );
  const exports = barrel.statements
    .filter(ts.isExportDeclaration)
    .flatMap((node) =>
      node.exportClause && ts.isNamedExports(node.exportClause)
        ? node.exportClause.elements.map((element) => element.name.text)
        : [],
    );
  assert.deepEqual(exports.sort(), components.sort());
});
