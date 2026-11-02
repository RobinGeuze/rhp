env REQUEST_METHOD=POST
env CONTENT_TYPE=Multipart/Form-Data; charset=x; BOUNDARY="XX"; x=y
stdin preamble\r\n--XX\r\nContent-Disposition: form-data; name="a"\r\n\r\n1\r\n--XX\nContent-Disposition: form-data; name="lf"\n\nlf only\n--XX--\r\nepilogue
