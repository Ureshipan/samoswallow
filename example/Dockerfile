# Tiny zero-dependency image: just the Python stdlib + our single-file app.
FROM python:3.12-alpine

WORKDIR /app
COPY app.py .

# The app honors the PORT env var (samoswallow injects/forwards it); default 8080.
EXPOSE 8080
CMD ["python", "app.py"]
