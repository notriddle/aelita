FROM alpine:edge
RUN apk add --no-cache postgresql-libs python2 py-pip py-psycopg2 uwsgi \
        uwsgi-python && \
    pip install flask GitHub-Flask Flask-SQLAlchemy raven 'raven[flask]'
COPY . /app
WORKDIR /app
CMD [ "uwsgi", "--plugin", "python", "-s", "0.0.0.0:8000", "--manage-script-name", "--mount", "/=main:app" ]
EXPOSE 8000

