# Supply the verified x86_64-unknown-linux-musl executable from the server CI job.
FROM scratch
COPY opensc-web /opensc-web
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/opensc-web"]
