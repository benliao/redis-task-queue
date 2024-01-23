# Redis Task Queue

A task queue based on redis using Rust language.

## Development Environment

### Test APIs

We utilize an extension of the vs code which called [&#34;REST Client&#34;](https://github.com/Huachao/vscode-restclient) to verify the APIs.

All the APIs call examples are in the file docs/api.rest.


### Cross Build Linux target from MacOS


```
# install pre build cross compiling tool
$ brew install SergioBenitez/osxct/x86_64-unknown-linux-gnu
```


```
podman run --platform linux/amd64 -v $(pwd)/target:/target --rm -ti redis/redis-stack-server:latest  /bin/bash
```
