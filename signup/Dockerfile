FROM alpine:edge
RUN apk add --no-cache postgresql-libs python2 py-pip py-psycopg2 && \
    pip install flask GitHub-Flask Flask-SQLAlchemy gunicorn
COPY . /app
WORKDIR /app
CMD [ "gunicorn", "-w", "2", "-b", "0.0.0.0:8000", "main:app" ]
EXPOSE 8000

