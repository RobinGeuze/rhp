env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
ini enable_post_data_reading=0
stdin --XX\r\nContent-Disposition: form-data; name="a"\r\n\r\n1\r\n--XX--\r\n
