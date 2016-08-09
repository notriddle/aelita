FROM alpine:edge
RUN apk add --no-cache postgresql-libs python2 py-pip py-psycopg2 && \
    pip install flask GitHub-Flask Flask-SQLAlchemy
COPY . /app
WORKDIR /app
CMD [ "/usr/local/bin/gunicorn", "-w", "2", "-b", ":8000", "main:app" ]
EXPOSE 8000

