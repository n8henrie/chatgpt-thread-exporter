{
  extension,
  gnumake,
  runCommand,
  zip,
}:

runCommand "${extension.pname}-${extension.version}-xpi" {
  nativeBuildInputs = [
    gnumake
    zip
  ];
} ''
  ln -s ${extension} extension
  make -f ${./Makefile} package
  install -Dm444 dist/chatgpt-thread-exporter-firefox.xpi "$out/chatgpt-thread-exporter-firefox.xpi"
''
