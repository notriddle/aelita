FROM python:3.4-alpine
COPY . /app
WORKDIR /app
RUN pip install -r requirements.txt
RUN pip install gunicorn
CMD [ "/usr/local/bin/gunicorn" "-w" "2" "-b" ":8000" "main:app" ]
EXPOSE 8000

