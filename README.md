# Redis Task Queue

A task queue based on redis using Rust language.

This project is inspired by a golang version redis queue api server. [https://github.com/bitleak/lmstfy]()

## Quick start

Copy .env.example to .env and update

Copy api-token.json.example to api-token.json and update accordingly.

Copy Containerfile.[yourhost] to Containerfile

Build images use docker or podman, then run it.

```
podman build -t redis-task-queue .
podman run -p 7766:7766 localhost/redis-task-queue
```

## Development Environment

### Test APIs

We utilize an extension of the vs code which called [&#34;REST Client&#34;](https://github.com/Huachao/vscode-restclient) to verify the APIs.

All the APIs call examples are in the file docs/api.rest.

### Cross Build Linux target from MacOS

If you are working in macos like me, please follow the below steps to cross build linux target.

```
# install pre build cross compiling tool
$ brew install SergioBenitez/osxct/x86_64-unknown-linux-gnu
```

 Run *cross_build.sh*
