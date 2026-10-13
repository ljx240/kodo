// Tiny CLI — no lodash use anywhere. See MEMORY.md.
function main(argv) {
  const out = argv.slice(2).map((s) => s.toUpperCase()).join(" ");
  console.log(out);
}
main(process.argv);
