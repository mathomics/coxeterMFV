FROM python:3.12-alpine

WORKDIR /app
COPY app/web/coxeter_mfv.html ./index.html
COPY LICENSES ./LICENSES
COPY LICENSE THIRD_PARTY_NOTICES.md ./

USER 10001:10001
EXPOSE 8080
CMD ["python", "-m", "http.server", "8080", "--bind", "0.0.0.0"]
